use std::{str::FromStr, sync::Arc};

use anyhow::{anyhow, Context, Result};
use bigdecimal::num_bigint::{BigInt, ToBigInt};
use bson::{doc, Bson, DateTime as BsonDateTime};
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
use bigdecimal::{BigDecimal, ToPrimitive};

lazy_static::lazy_static! {
    static ref RENEW_TIME: FieldElement = FieldElement::from_dec_str("365").unwrap();
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

fn to_uint256(n: BigInt) -> (FieldElement, FieldElement) {
    let two_pow_128 = BigInt::from(2).pow(128);

    let low = (&n % &two_pow_128).to_u128().unwrap().to_string();
    let high = (n / two_pow_128).to_u128().unwrap().to_string();

    (
        FieldElement::from_dec_str(&low).unwrap(),
        FieldElement::from_dec_str(&high).unwrap(),
    )
}

pub async fn get_domains_ready_for_renewal(
    config: &Config,
    state: &Arc<AppState>,
) -> Result<(Vec<FieldElement>, Vec<FieldElement>, Vec<BigDecimal>)> {
    let domains = state.db.collection::<Domain>("domains");
    let min_expiry_date = Utc::now() + Duration::days(30);

    // Define aggregate pipeline
    let pipeline = vec![
        doc! { "$match": { "_chain.valid_to": { "$exists": Bson::Null } } },
        doc! { "$match": { "expiry": { "$lt": BsonDateTime::from_millis(min_expiry_date.timestamp_millis()) } } },
        doc! { "$lookup": {
            "from": "auto_renewals",
            "let": { "domain_name": "$domain" },
            "pipeline": [
                { "$match":
                    { "$expr":
                        { "$and": [
                            { "$eq": [ "$domain",  "$$domain_name" ] },
                            { "$eq": [ "$_chain.valid_to",  Bson::Null ] }
                        ]}
                    }
                }
            ],
            "as": "renewal_info",
        }},
        doc! { "$unwind": "$renewal_info" },
        doc! { "$lookup": {
            "from": "approvals",
            "let": { "renewer_addr": "$renewal_info.renewer_address" },
            "pipeline": [
                { "$match":
                    { "$expr":
                        { "$and": [
                            { "$eq": [ "$renewer",  "$$renewer_addr" ] },
                            { "$eq": [ "$_chain.valid_to",  Bson::Null ] }
                        ]}
                    }
                }
            ],
            "as": "approval_info",
        }},
        doc! { "$unwind": { "path": "$approval_info", "preserveNullAndEmptyArrays": true } },
        doc! { "$project": {
            "domain": 1,
            "expiry": 1,
            "renewer_address": "$renewal_info.renewer_address",
            "auto_renewal_enabled": "$renewal_info.auto_renewal_enabled",
            "approval_value": { "$ifNull": [ "$approval_info.value", "0" ] },
            "limit_price": "$renewal_info.limit_price",
            "last_renewal": "$renewal_info.last_renewal",
            "_chain": "$renewal_info._chain",
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
    let (domains_ready, renewer_limit_tuples): (
        Vec<FieldElement>,
        Vec<(FieldElement, BigDecimal)>,
    ) = processed_results.into_iter().unzip();
    let (renewer_ready, limit_price_ready): (Vec<FieldElement>, Vec<BigDecimal>) =
        renewer_limit_tuples.into_iter().unzip();
    Ok((domains_ready, renewer_ready, limit_price_ready))
}

async fn process_aggregate_result(
    result: DomainAggregateResult,
    config: &Config,
) -> Result<Option<(FieldElement, (FieldElement, BigDecimal))>> {
    // Skip the rest if auto-renewal is not enabled
    if !result.auto_renewal_enabled {
        return Ok(None);
    }

    let renewer_addr = FieldElement::from_str(&result.renewer_address)?;
    let allowance = BigDecimal::from_str(&result.approval_value)?;
    let limit_price = BigDecimal::from_str(&result.limit_price)?;

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
            if renew_price <= allowance && renew_price <= limit_price {
                Ok(Some((domain_encoded, (renewer_addr, limit_price))))
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
    domains: (Vec<FieldElement>, Vec<FieldElement>, Vec<BigDecimal>),
) -> Result<()> {
    // If we have more than 400 domains to renew we make multiple transactions to avoid hitting the 2M steps limit
    while !domains.0.is_empty() && !domains.1.is_empty() && !domains.2.is_empty() {
        let mut domains = domains.clone();
        let size = domains.0.len().min(400);
        let domains_to_renew: Vec<FieldElement> = domains.0.drain(0..size).collect();
        let renewers = domains.1.drain(0..size).collect();
        let limit_prices = domains.2.drain(0..size).collect();

        match send_transaction(
            config,
            account,
            (domains_to_renew.clone(), renewers, limit_prices),
        )
        .await
        {
            Ok(_) => {
                log_msg_and_send_to_discord(
                    &config,
                    "[bot][renew_domains]",
                    &format!("Successfully renewed domains: {:?}", domains_to_renew),
                )
                .await
            }
            Err(e) => {
                log_msg_and_send_to_discord(
                    &config,
                    "[bot][renew_domains]",
                    &format!(
                        "Error while renewing domains: {:?} for domains: {:?}",
                        e, domains_to_renew
                    ),
                )
                .await;
                return Err(e);
            }
        }
    }
    Ok(())
}

pub async fn send_transaction(
    config: &Config,
    account: &SingleOwnerAccount<SequencerGatewayProvider, LocalWallet>,
    domains: (Vec<FieldElement>, Vec<FieldElement>, Vec<BigDecimal>),
) -> Result<()> {
    let mut calldata: Vec<FieldElement> = Vec::new();
    calldata.push(FieldElement::from_dec_str(&domains.0.len().to_string()).unwrap());
    calldata.extend_from_slice(&domains.0);
    calldata.push(FieldElement::from_dec_str(&domains.1.len().to_string()).unwrap());
    calldata.extend_from_slice(&domains.1);
    calldata.push(FieldElement::from_dec_str(&domains.2.len().to_string()).unwrap());

    for limit_price in &domains.2 {
        let (low, high) = to_uint256(limit_price.to_bigint().unwrap());
        calldata.push(low);
        calldata.push(high);
    }

    let result = account
        .execute(vec![Call {
            to: config.contract.renewal,
            selector: selector!("batch_renew"),
            calldata,
        }])
        .send()
        .await;

    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            let error_message = format!("An error occurred while renewing domains: {}", e);
            Err(anyhow::anyhow!(error_message))
        }
    }
}
