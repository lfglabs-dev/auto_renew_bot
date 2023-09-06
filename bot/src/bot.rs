use std::{str::FromStr, sync::Arc};

use anyhow::{anyhow, Context, Result};
use bigdecimal::num_bigint::{BigInt, ToBigInt};
use bson::{doc, Bson};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::options::FindOneOptions;
use num_integer::Integer;
use starknet::accounts::Account;
use starknet::core::chain_id;
use starknet::{
    accounts::{Call, SingleOwnerAccount},
    core::types::{BlockId, CallFunction, FieldElement},
    macros::selector,
    providers::{Provider, SequencerGatewayProvider},
    signers::LocalWallet,
};
use url::Url;

use crate::discord::log_msg_and_send_to_discord;
use crate::models::{
    AggregateResult, AggregateResults, DomainAggregateResult, MetadataDoc, Unzip5,
};
use crate::{
    config::Config,
    models::{AppState, Domain},
};
use bigdecimal::{BigDecimal, FromPrimitive};

lazy_static::lazy_static! {
    static ref RENEW_TIME: FieldElement = FieldElement::from_dec_str("365").unwrap();
    static ref TWO_POW_128: BigInt = BigInt::from(2).pow(128);
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

pub fn get_chainid(config: &Config) -> FieldElement {
    if config.devnet_provider.is_devnet || config.devnet_provider.is_testnet {
        chain_id::TESTNET
    } else {
        chain_id::MAINNET
    }
}

fn to_uint256(n: BigInt) -> (FieldElement, FieldElement) {
    let (n_high, n_low) = n.div_rem(&TWO_POW_128);
    let (_, low_bytes) = n_low.to_bytes_be();
    let (_, high_bytes) = n_high.to_bytes_be();

    (
        FieldElement::from_byte_slice_be(&low_bytes).unwrap(),
        FieldElement::from_byte_slice_be(&high_bytes).unwrap(),
    )
}

pub async fn get_domains_ready_for_renewal(
    config: &Config,
    state: &Arc<AppState>,
) -> Result<AggregateResults> {
    let domains = state.db.collection::<Domain>("domains");
    let min_expiry_date = Utc::now() + Duration::days(30);

    // Define aggregate pipeline
    let pipeline = vec![
        doc! { "$match": { "_chain.valid_to": Bson::Null } },
        doc! { "$match": { "expiry": { "$lt": Bson::Int32((min_expiry_date.timestamp_millis() / 1000).try_into().unwrap()) } } },
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
            "meta_hash": "$renewal_info.meta_hash",
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
        .map(|result| process_aggregate_result(state, result, config))
        .collect();

    let processed_results: Vec<_> = futures::future::try_join_all(futures)
        .await?
        .into_iter()
        .flatten()
        .collect();

    let (domains, renewers, limit_prices, tax_prices, meta_hashes): (
        Vec<FieldElement>,
        Vec<FieldElement>,
        Vec<BigDecimal>,
        Vec<BigDecimal>,
        Vec<FieldElement>,
    ) = processed_results
        .into_iter()
        .map(|res| {
            (
                res.domain,
                res.renewer_addr,
                res.limit_price,
                res.tax_price,
                res.meta_hash,
            )
        })
        .unzip5();
    Ok(AggregateResults {
        domains,
        renewers,
        limit_prices,
        tax_prices,
        meta_hashes,
    })
}

async fn process_aggregate_result(
    state: &Arc<AppState>,
    result: DomainAggregateResult,
    config: &Config,
) -> Result<Option<AggregateResult>> {
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
                if result.meta_hash != "0" {
                    let decimal_val =
                        BigInt::parse_bytes(result.meta_hash.as_str().as_bytes(), 10).unwrap();
                    let hex_meta_hash = decimal_val.to_str_radix(16);
                    let metadata_collection =
                        state.db_metadata.collection::<MetadataDoc>("metadata");
                    if let Some(document) = metadata_collection
                        .find_one(doc! {"meta_hash": hex_meta_hash}, FindOneOptions::default())
                        .await?
                    {
                        let tax_state = document.tax_state;
                        if let Some(state_info) = state.states.states.get(&tax_state) {
                            let tax_rate = (state_info.rate * 100.0).round() as i32;
                            let tax_price =
                                (renew_price * BigDecimal::from(tax_rate)) / BigDecimal::from(100);
                            return Ok(Some(AggregateResult {
                                domain: domain_encoded,
                                renewer_addr,
                                limit_price,
                                tax_price,
                                meta_hash: FieldElement::from_dec_str(&result.meta_hash)?,
                            }));
                        }
                    }
                }
                Ok(Some(AggregateResult {
                    domain: domain_encoded,
                    renewer_addr,
                    limit_price,
                    tax_price: BigDecimal::from(0),
                    meta_hash: FieldElement::from_str("0")?,
                }))
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
            "Error while fetching renew price for domain {:?} : {:?}",
            &result.domain,
            e
        )),
    }
}

pub async fn renew_domains(
    config: &Config,
    account: &SingleOwnerAccount<SequencerGatewayProvider, LocalWallet>,
    mut aggregate_results: AggregateResults,
) -> Result<()> {
    // If we have more than 400 domains to renew we make multiple transactions to avoid hitting the 2M steps limit
    while !aggregate_results.domains.is_empty()
        && !aggregate_results.renewers.is_empty()
        && !aggregate_results.limit_prices.is_empty()
        && !aggregate_results.tax_prices.is_empty()
        && !aggregate_results.meta_hashes.is_empty()
    {
        let size = aggregate_results.domains.len().min(400);
        let domains_to_renew: Vec<FieldElement> =
            aggregate_results.domains.drain(0..size).collect();
        let renewers: Vec<FieldElement> = aggregate_results.renewers.drain(0..size).collect();
        let limit_prices: Vec<BigDecimal> = aggregate_results.limit_prices.drain(0..size).collect();
        let tax_prices: Vec<BigDecimal> = aggregate_results.tax_prices.drain(0..size).collect();
        let meta_hashes: Vec<FieldElement> = aggregate_results.meta_hashes.drain(0..size).collect();

        match send_transaction(
            config,
            account,
            AggregateResults {
                domains: domains_to_renew.clone(),
                renewers,
                limit_prices,
                tax_prices,
                meta_hashes,
            },
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
    aggregate_results: AggregateResults,
) -> Result<()> {
    let mut calldata: Vec<FieldElement> = Vec::new();
    calldata
        .push(FieldElement::from_dec_str(&aggregate_results.domains.len().to_string()).unwrap());
    calldata.extend_from_slice(&aggregate_results.domains);
    calldata
        .push(FieldElement::from_dec_str(&aggregate_results.renewers.len().to_string()).unwrap());
    calldata.extend_from_slice(&aggregate_results.renewers);
    calldata.push(
        FieldElement::from_dec_str(&aggregate_results.limit_prices.len().to_string()).unwrap(),
    );

    for limit_price in &aggregate_results.limit_prices {
        let (low, high) = to_uint256(limit_price.to_bigint().unwrap());
        calldata.push(low);
        calldata.push(high);
    }

    calldata
        .push(FieldElement::from_dec_str(&aggregate_results.tax_prices.len().to_string()).unwrap());
    for tax_price in &aggregate_results.tax_prices {
        let (low, high) = to_uint256(tax_price.to_bigint().unwrap());
        calldata.push(low);
        calldata.push(high);
    }
    calldata.push(
        FieldElement::from_dec_str(&aggregate_results.meta_hashes.len().to_string()).unwrap(),
    );
    calldata.extend_from_slice(&aggregate_results.meta_hashes);

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
