use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use anyhow::{anyhow, Context, Result};
use bigdecimal::{
    num_bigint::{BigInt, ToBigInt},
    BigDecimal,
};
use bson::{doc, Bson};
use chrono::{Duration, Utc};
use futures::stream::{self, StreamExt};
use futures::TryStreamExt;
use mongodb::options::FindOneOptions;
use starknet::accounts::ConnectedAccount;
use starknet::core::types::{BlockTag, FunctionCall};
use starknet::{
    accounts::{Account, Call, SingleOwnerAccount},
    core::types::{BlockId, FieldElement},
    macros::selector,
    providers::{jsonrpc::HttpTransport, JsonRpcClient, Provider},
    signers::LocalWallet,
};
use starknet_id::encode;
use tokio::time::{sleep, Duration as TokioDuration};

use crate::logger::Logger;
use crate::models::TxResult;
use crate::models::{
    AggregateResult, AggregateResults, DomainAggregateResult, MetadataDoc, Unzip5,
};
use crate::starknet_utils::check_pending_transactions;
use crate::starknet_utils::create_jsonrpc_client;
use crate::starknetid_utils::get_renewal_price;
use crate::utils::{from_uint256, hex_to_bigdecimal, to_uint256};
use crate::{
    config::Config,
    models::{AppState, Domain},
};

lazy_static::lazy_static! {
    static ref RENEW_TIME: FieldElement = FieldElement::from_dec_str("365").unwrap();
}

pub async fn get_domains_ready_for_renewal(
    config: &Config,
    state: &Arc<AppState>,
    logger: &Logger,
) -> Result<AggregateResults> {
    let auto_renews_collection = state.db.collection::<Domain>("auto_renew_flows");
    let min_expiry_date = Utc::now() + Duration::days(30);
    println!("timestamp: {}", min_expiry_date.timestamp());
    // Define aggregate pipeline
    let pipeline = vec![
        doc! { "$match": { "_cursor.to": null } },
        doc! { "$match": { "enabled": true } },
        doc! { "$lookup": {
            "from": "domains",
            "let": { "domain_name": "$domain" },
            "pipeline": [
                { "$match":
                    { "$expr":
                        { "$and": [
                            { "$eq": [ "$domain",  "$$domain_name" ] },
                            { "$eq": [ { "$ifNull": [ "$_cursor.to", null ] }, null ] },
                        ]}
                    }
                },
            ],
            "as": "domain_info",
        }},
        doc! { "$unwind": "$domain_info" },
        doc! { "$match": { "domain_info.expiry": { "$lt": Bson::Int64(min_expiry_date.timestamp()) } } },
        doc! { "$lookup": {
            "from": "auto_renew_approvals",
            "let": { "renewer_addr": "$renewer_address" },
            "pipeline": [
                { "$match":
                    { "$expr":
                        { "$and": [
                            { "$eq": [ "$renewer",  "$$renewer_addr" ] },
                            { "$eq": [ { "$ifNull": [ "$_cursor.to", null ] }, null ] },
                        ]}
                    }
                }
            ],
            "as": "approval_info",
        }},
        doc! { "$unwind": { "path": "$approval_info", "preserveNullAndEmptyArrays": true } },
        doc! { "$group": {
            "_id": "$domain_info.domain",
            "expiry": { "$first": "$domain_info.expiry" },
            "renewer_address": { "$first": "$renewer_address" },
            "enabled": { "$first": "$enabled" },
            "approval_value": { "$first": { "$ifNull": [ "$approval_info.allowance", "0x0" ] } },
            "allowance": { "$first": "$allowance" },
            "last_renewal": { "$first": "$last_renewal" },
            "meta_hash": { "$first": "$meta_hash" },
            "_cursor": { "$first": "$_cursor" },
        }},
        doc! { "$project": {
            "_id": 0,
            "domain": "$_id",
            "expiry": 1,
            "renewer_address": 1,
            "enabled": 1,
            "approval_value": 1,
            "allowance": 1,
            "last_renewal": 1,
            "meta_hash": 1,
            "_cursor": 1,
        }},
    ];

    // Execute the pipeline
    let cursor = auto_renews_collection.aggregate(pipeline, None).await?;
    // Extract the results as Vec<bson::Document>
    let bson_docs: Vec<bson::Document> = cursor.try_collect().await?;
    // Convert each bson::Document into DomainAggregateResult
    let results: Result<Vec<DomainAggregateResult>, _> = bson_docs
        .into_iter()
        .map(|doc| bson::from_bson(bson::Bson::Document(doc)))
        .collect();
    // Check if the conversion was successful
    let results = results?;

    if results.is_empty() {
        return Ok(AggregateResults {
            domains: vec![],
            renewers: vec![],
            domain_prices: vec![],
            tax_prices: vec![],
            meta_hashes: vec![],
        });
    }

    // Fetch balances for all renewers
    let renewer_addresses: Vec<String> = results
        .iter()
        .map(|result| result.renewer_address.clone())
        .collect();
    // Batch calls to fetch balances to max 5000 addresses per call
    let mut balances: Vec<FieldElement> = vec![];
    let mut renewer_addresses_batch = renewer_addresses.clone();
    while !renewer_addresses_batch.is_empty() {
        let size = renewer_addresses_batch.len().min(2500);
        let batch = renewer_addresses_batch.drain(0..size).collect();
        let batch_balances = fetch_users_balances(config, batch).await;
        // we skip the first 2 elements, the first one is the index of the call, the 2nd the length of the results
        balances.extend(batch_balances.into_iter().skip(2));
    }
    println!("balances at the end: {:?}", balances.len());
    if balances.is_empty() {
        logger.severe(format!(
            "Error while fetching balances for {} users",
            renewer_addresses.len()
        ));
        return Err(anyhow!("Error while fetching balances"));
    }

    let dynamic_balances = Arc::new(Mutex::new(HashMap::new()));
    let mut balances_iter = balances.into_iter();
    for address in &renewer_addresses {
        balances_iter.next(); // we skip the first result as its value is 2 for low and high
        let balance = from_uint256(
            balances_iter.next().expect("Expected low not found"),
            balances_iter.next().expect("Expected high not found"),
        );
        dynamic_balances
            .lock()
            .unwrap()
            .insert(address.to_owned(), balance);
    }

    // Then process the results
    let results_stream = stream::iter(results.into_iter().enumerate());
    let processed_results = results_stream
        .then(|(i, result)| {
            let address = renewer_addresses.get(i).unwrap();
            let balance = dynamic_balances
                .lock()
                .unwrap()
                .get(address)
                .unwrap()
                .to_owned();
            let renewal_price = get_renewal_price(result.domain.clone());
            let dynamic_balances = Arc::clone(&dynamic_balances);
            async move {
                let output = process_aggregate_result(
                    state,
                    result,
                    logger,
                    BigDecimal::from(balance.to_owned()),
                    BigDecimal::from(renewal_price.to_owned()),
                )
                .await;

                if output.is_some() {
                    let new_balance = balance - renewal_price;
                    dynamic_balances
                        .lock()
                        .unwrap()
                        .insert(address.to_owned(), new_balance);
                };

                output
            }
        })
        .collect::<Vec<_>>()
        .await;

    let mut none_count = 0;
    let (domains, renewers, domain_prices, tax_prices, meta_hashes): (
        Vec<FieldElement>,
        Vec<FieldElement>,
        Vec<BigDecimal>,
        Vec<BigDecimal>,
        Vec<FieldElement>,
    ) = processed_results
        .into_iter()
        .filter_map(|x| match x {
            Some(res) => Some(res),
            None => {
                none_count += 1;
                None
            }
        })
        .map(|res| {
            (
                res.domain,
                res.renewer_addr,
                res.domain_price,
                res.tax_price,
                res.meta_hash,
            )
        })
        .unzip5();
    logger.warning(format!("Domains that couldn't be renewed: {}", none_count));

    Ok(AggregateResults {
        domains,
        renewers,
        domain_prices,
        tax_prices,
        meta_hashes,
    })
}

async fn fetch_users_balances(
    config: &Config,
    renewer_addresses: Vec<String>,
) -> Vec<FieldElement> {
    let mut calls: Vec<FieldElement> = vec![FieldElement::from(renewer_addresses.len())];
    for address in &renewer_addresses {
        calls.push(config.contract.erc20);
        calls.push(selector!("balanceOf"));
        calls.push(FieldElement::ONE);
        calls.push(FieldElement::from_hex_be(address).unwrap());
    }

    let provider = create_jsonrpc_client(&config);
    let call_result = provider
        .call(
            FunctionCall {
                contract_address: config.contract.multicall,
                entry_point_selector: selector!("aggregate"),
                calldata: calls,
            },
            BlockId::Tag(BlockTag::Latest),
        )
        .await;

    match call_result {
        Ok(result) => result,
        Err(err) => {
            println!("Error while fetching balances: {:?}", err);
            vec![]
        }
    }
}

async fn process_aggregate_result(
    state: &Arc<AppState>,
    result: DomainAggregateResult,
    _logger: &Logger,
    balance: BigDecimal,
    renewal_price: BigDecimal,
) -> Option<AggregateResult> {
    // Skip the rest if auto-renewal is not enabled
    if !result.enabled || result.allowance.is_none() {
        return None;
    }

    let renewer_addr = FieldElement::from_hex_be(&result.renewer_address).unwrap();
    let erc20_allowance = if let Some(approval_value) = result.approval_value {
        hex_to_bigdecimal(&approval_value).unwrap()
    } else {
        BigDecimal::from(0)
    };
    let allowance = hex_to_bigdecimal(&result.allowance.unwrap()).unwrap();

    // Check user meta hash
    let mut tax_price = BigDecimal::from(0);
    let mut meta_hash = FieldElement::ZERO;
    if let Some(hash) = result.meta_hash {
        meta_hash = FieldElement::from_hex_be(&hash).unwrap();
        if hash != "0" {
            let decimal_meta_hash =
                BigInt::parse_bytes(hash.trim_start_matches("0x").as_bytes(), 16).unwrap();
            let hex_meta_hash = decimal_meta_hash.to_str_radix(16);
            let metadata_collection = state.db_metadata.collection::<MetadataDoc>("metadata");
            if let Ok(Some(document)) = metadata_collection
                .find_one(doc! {"meta_hash": hex_meta_hash}, FindOneOptions::default())
                .await
            {
                let tax_state = document.tax_state;
                if let Some(state_info) = state.states.states.get(&tax_state) {
                    let tax_rate = (state_info.rate * 100.0).round() as i32;
                    tax_price = (renewal_price.clone() * BigDecimal::from(tax_rate))
                        / BigDecimal::from(100);
                }
            }
        }
    }
    let final_price = renewal_price.clone() + tax_price.clone();

    // Check user ETH allowance is greater or equal than final price = renew_price + tax_price
    if erc20_allowance >= final_price {
        // check user allowance is greater or equal than final price
        if allowance >= final_price {
            // check user balance is sufficiant
            if balance < final_price {
                // logger.warning(format!(
                //     "Domain {} cannot be renewed because {} has not enough balance ({}) for domain price({})",
                //     result.domain, result.renewer_address, balance, final_price
                // ));
                return None;
            }

            // encode domain name
            let domain_name = result
                .domain
                .strip_suffix(".stark")
                .ok_or_else(|| anyhow::anyhow!("Invalid domain name: {:?}", result.domain))
                .unwrap();
            let domain_encoded = encode(domain_name)
                .map_err(|_| anyhow!("Failed to encode domain name"))
                .context("Error occurred while encoding domain name")
                .unwrap();
            println!(
                "[OK] Domain {}.stark can be renewed by {}",
                domain_name, renewer_addr
            );
            Some(AggregateResult {
                domain: domain_encoded,
                renewer_addr,
                domain_price: renewal_price,
                tax_price,
                meta_hash,
            })
        } else {
            // logger.warning(format!(
            //     "Domain {} cannot be renewed because {} has set an allowance({}) lower than final price({})",
            //     result.domain, result.renewer_address, allowance, final_price
            // ));
            None
        }
    } else {
        // logger.warning(format!(
        //     "Domain {} cannot be renewed because {} has set an erc20_allowance ({}) lower than domain price({}) + tax({})",
        //     result.domain, result.renewer_address, erc20_allowance, renewal_price, tax_price
        // ));
        None
    }
}

pub async fn renew_domains(
    config: &Config,
    account: &SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    mut aggregate_results: AggregateResults,
    logger: &Logger,
) -> Result<()> {
    logger.info(format!(
        "Renewing {} domains",
        aggregate_results.domains.len()
    ));
    let mut nonce = account.get_nonce().await.unwrap();
    let mut tx_results = Vec::<TxResult>::new();

    // If we have: i32 more than 75 domains to renew we make multiple transactions to avoid hitting the 3M steps limit
    while !aggregate_results.domains.is_empty()
        && !aggregate_results.renewers.is_empty()
        && !aggregate_results.domain_prices.is_empty()
        && !aggregate_results.tax_prices.is_empty()
        && !aggregate_results.meta_hashes.is_empty()
    {
        let size = aggregate_results.domains.len().min(75);
        let domains_to_renew: Vec<FieldElement> =
            aggregate_results.domains.drain(0..size).collect();
        let renewers: Vec<FieldElement> = aggregate_results.renewers.drain(0..size).collect();
        let domain_prices: Vec<BigDecimal> =
            aggregate_results.domain_prices.drain(0..size).collect();
        let tax_prices: Vec<BigDecimal> = aggregate_results.tax_prices.drain(0..size).collect();
        let meta_hashes: Vec<FieldElement> = aggregate_results.meta_hashes.drain(0..size).collect();

        match send_transaction(
            config,
            account,
            AggregateResults {
                domains: domains_to_renew.clone(),
                renewers,
                domain_prices,
                tax_prices,
                meta_hashes,
            },
            nonce,
        )
        .await
        {
            Ok(tx_hash) => {
                logger.info(format!(
                    "Sent a tx 0x{:x} to renew {:} domains with nonce: {}",
                    &tx_hash,
                    domains_to_renew.len(),
                    nonce,
                ));
                tx_results.push(TxResult {
                    tx_hash,
                    reverted: None,
                    revert_reason: None,
                    domains_renewed: domains_to_renew.len(),
                });

                // We only inscrease nonce if no error occurred in the previous transaction
                nonce += FieldElement::ONE;
            }
            Err(e) => {
                if e.to_string().contains("Error while estimating fee") {
                    logger.info(format!(
                        "Error while estimating fees : {:?} for domains: {:?}",
                        e, domains_to_renew
                    ));
                    logger.info("Continuing with the next transaction...");
                    continue;
                } else {
                    logger.severe(format!(
                        "Error while renewing domains: {:?} for domains: {:?}",
                        e,
                        domains_to_renew.len()
                    ));
                    return Err(e);
                }
            }
        }
        
        check_pending_transactions(config, &mut tx_results).await;

        let filtered_results: Vec<&TxResult> = tx_results
            .iter()
            .filter(|tx| tx.reverted.is_some())
            .collect();

        let failed_count = filtered_results
            .iter()
            .rev()
            .take(3)
            .filter(|tx| tx.reverted == Some(true))
            .count();

        // If 3 transactions have failed, we stop the process
        if failed_count == 3 {
            logger.severe("The last 3 transactions have failed. Stopping process.");
            logger.info(format!("Sent {:?} transactions", tx_results.len()));
            filtered_results.iter().rev().take(3).for_each(|failure| {
                logger.severe(format!(
                    "Transaction 0x{:x} with {:?} domains has failed with reason: {:?}",
                    failure.tx_hash, failure.domains_renewed, failure.revert_reason
                ));
            });
            logger.severe("Stopping process.");
            break;
        }

        println!("Waiting for 1 minute before sending the next transaction...");
        sleep(TokioDuration::from_secs(60)).await;
    }
    Ok(())
}

pub async fn send_transaction(
    config: &Config,
    account: &SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    aggregate_results: AggregateResults,
    nonce: FieldElement,
) -> Result<FieldElement> {
    let mut calldata: Vec<FieldElement> = Vec::new();
    calldata
        .push(FieldElement::from_dec_str(&aggregate_results.domains.len().to_string()).unwrap());
    calldata.extend_from_slice(&aggregate_results.domains);
    calldata
        .push(FieldElement::from_dec_str(&aggregate_results.renewers.len().to_string()).unwrap());
    calldata.extend_from_slice(&aggregate_results.renewers);
    calldata.push(
        FieldElement::from_dec_str(&aggregate_results.domain_prices.len().to_string()).unwrap(),
    );

    println!("domains:");
    for x in &aggregate_results.domains {
        println!("{}", x);
    }

    println!("renewers:");
    for x in &aggregate_results.renewers {
        println!("{}", x);
    }

    println!("domain_prices:");
    for x in &aggregate_results.domain_prices {
        println!("{}", x);
    }

    println!("tax_prices:");
    for x in &aggregate_results.tax_prices {
        println!("{}", x);
    }

    println!("meta_hashes:");
    for x in &aggregate_results.meta_hashes {
        println!("{}", x);
    }

    for limit_price in &aggregate_results.domain_prices {
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

    let execution = account
        .execute(vec![Call {
            to: config.contract.renewal,
            selector: selector!("batch_renew"),
            calldata: calldata.clone(),
        }])
        .fee_estimate_multiplier(1.5f64);

    match execution.estimate_fee().await {
        Ok(fee) => match execution.nonce(nonce).max_fee(fee.overall_fee).send().await {
            Ok(tx_result) => Ok(tx_result.transaction_hash),
            Err(e) => {
                let error_message = format!("An error occurred while renewing domains: {}", e);
                Err(anyhow::anyhow!(error_message))
            }
        },
        Err(e) => {
            println!("Error while estimating fee: {:?}", e);
            let error_message = format!("Error while estimating fee: {}", e);
            Err(anyhow::anyhow!(error_message))
        }
    }
}
