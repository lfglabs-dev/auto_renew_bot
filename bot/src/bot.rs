use std::{str::FromStr, sync::Arc};

use anyhow::{anyhow, Context, Result};
use bson::{doc, DateTime as BsonDateTime};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use starknet::accounts::Account;
use starknet::{
    accounts::{Call, SingleOwnerAccount},
    core::types::{BlockId, CallFunction, FieldElement},
    macros::selector,
    providers::{Provider, SequencerGatewayProvider},
    signers::LocalWallet,
};
use url::Url;

use crate::discord::log_msg_and_send_to_discord;
use crate::models::DomainAggregateResult;
use crate::{
    config::Config,
    models::{AppState, Domain},
};
use bigdecimal::BigDecimal;

lazy_static::lazy_static! {
    static ref RENEW_TIME: FieldElement = FieldElement::from_dec_str("60").unwrap();
}

pub fn get_provider(config: &Config) -> SequencerGatewayProvider {
    if config.devnet_provider.is_devnet {
        SequencerGatewayProvider::new(
            Url::from_str(&config.devnet_provider.gateway).unwrap(),
            Url::from_str(&config.devnet_provider.feeder_gateway).unwrap(),
        )
    } else if config.devnet_provider.is_testnet {
        SequencerGatewayProvider::starknet_alpha_goerli()
    } else {
        SequencerGatewayProvider::starknet_alpha_mainnet()
    }
}

pub async fn get_domains_ready_for_renewal(
    config: &Config,
    state: &Arc<AppState>,
) -> Result<(Vec<FieldElement>, Vec<FieldElement>)> {
    let domains = state.db.collection::<Domain>("domains");
    // todo: for testing purposes duration is set to 120 days, otherwhise should be set as 30 days
    let min_expiry_date = Utc::now() + Duration::days(120);

    // Define aggregate pipeline
    let pipeline = vec![
        doc! { "$match": { "expiry": { "$lt": BsonDateTime::from_millis(min_expiry_date.timestamp_millis()) } } },
        doc! { "$lookup": {
            "from": "auto_renewals",
            "localField": "domain",
            "foreignField": "domain",
            "as": "renewal_info",
        }},
        doc! { "$unwind": "$renewal_info" },
        doc! { "$lookup": {
            "from": "approvals",
            "localField": "renewal_info.renewer_address",
            "foreignField": "renewer",
            "as": "approval_info",
        }},
        doc! { "$unwind": { "path": "$approval_info", "preserveNullAndEmptyArrays": true } },
        doc! { "$project": {
            "domain": 1,
            "expiry": 1,
            "renewer_address": "$renewal_info.renewer_address",
            "auto_renewal_enabled": "$renewal_info.auto_renewal_enabled",
            "approval_value": { "$ifNull": [ "$approval_info.value", "0" ] },
        }},
    ];

    // Execute the pipeline
    let cursor = domains.aggregate(pipeline, None).await?;
    // Extract the results as Vec<bson::Document>
    let bson_docs: Vec<bson::Document> = cursor.try_collect().await?;
    // Convert each bson::Document into DomainAggregateResult
    let results: Result<Vec<DomainAggregateResult>, _> = bson_docs
        .into_iter()
        .map(|doc| bson::from_bson(bson::Bson::Document(doc)))
        .collect();
    // Check if the conversion was successful
    let results = results?;

    // Then process the results
    let futures: Vec<_> = results
        .into_iter()
        .map(|result| process_aggregate_result(result, config))
        .collect();
    let processed_results: Vec<_> = futures::future::try_join_all(futures)
        .await?
        .into_iter()
        .flatten()
        .collect();
    let (domains_ready, renewer_ready): (Vec<_>, Vec<_>) = processed_results.into_iter().unzip();
    Ok((domains_ready, renewer_ready))
}

async fn process_aggregate_result(
    result: DomainAggregateResult,
    config: &Config,
) -> Result<Option<(FieldElement, FieldElement)>> {
    // Skip the rest if auto-renewal is not enabled
    if !result.auto_renewal_enabled {
        return Ok(None);
    }

    let renewer_addr = FieldElement::from_str(&result.renewer_address)?;
    let allowance = BigDecimal::from_str(&result.approval_value)?;

    // get renew price from contract
    let provider = get_provider(&config);
    let domain_name = result
        .domain
        .strip_suffix(".stark")
        .ok_or_else(|| anyhow::anyhow!("Invalid domain name: {:?}", result.domain))?;
    let domain_encoded = starknet::id::encode(domain_name)
        .map_err(|_| anyhow!("Failed to encode domain name"))
        .context("Error occurred while encoding domain name")?;

    let call_result = provider
        .call_contract(
            CallFunction {
                contract_address: config.contract.pricing,
                entry_point_selector: selector!("compute_renew_price"),
                calldata: vec![domain_encoded, *RENEW_TIME],
            },
            BlockId::Latest,
        )
        .await;

    match call_result {
        Ok(res) => {
            let renew_price = FieldElement::to_big_decimal(&res.result[1], 18);
            if renew_price <= allowance {
                Ok(Some((domain_encoded, renewer_addr)))
            } else {
                log_msg_and_send_to_discord(
                    &config,
                    "[Renewal]",
                    &format!(
                        "Domain {} cannot be renewed because {} has not enough allowance",
                        result.domain, result.renewer_address
                    ),
                )
                .await;
                Ok(None)
            }
        }
        Err(e) => Err(anyhow::anyhow!(
            "Error while fetching renew priced for domain {:?} : {:?}",
            &result.domain,
            e
        )),
    }
}

pub async fn renew_domains(
    config: &Config,
    account: &SingleOwnerAccount<SequencerGatewayProvider, LocalWallet>,
    domains: (Vec<FieldElement>, Vec<FieldElement>),
) -> Result<()> {
    let mut calldata: Vec<FieldElement> = Vec::new();
    calldata.push(FieldElement::from_dec_str(&domains.0.len().to_string()).unwrap());
    calldata.extend_from_slice(&domains.0);
    calldata.push(FieldElement::from_dec_str(&domains.1.len().to_string()).unwrap());
    calldata.extend_from_slice(&domains.1);

    println!("Renewing domains with calldata: {:?}", calldata);
    println!("contract addr renewal: {:?}", config.contract.renewal);

    let result = account
        .execute(vec![Call {
            to: config.contract.renewal,
            selector: selector!("batch_renew"),
            calldata,
        }])
        .send()
        .await;
    println!("batch_renew execute result : {:?}", result);
    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            let error_message = format!("An error occurred while renewing domains: {}", e);
            Err(anyhow::anyhow!(error_message))
        }
    }
}
