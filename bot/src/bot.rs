use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use anyhow::{anyhow, Context, Result};
use bigdecimal::{
    num_bigint::{BigInt, ToBigInt},
    BigDecimal,
};
use bson::doc;
use futures::stream::{self, StreamExt};
use mongodb::options::FindOneOptions;
use starknet::accounts::ConnectedAccount;
use starknet::{
    accounts::{Account, Call, SingleOwnerAccount},
    core::types::FieldElement,
    macros::selector,
    providers::{jsonrpc::HttpTransport, JsonRpcClient},
    signers::LocalWallet,
};
use starknet_id::encode;
use std::str::FromStr;
use tokio::time::{sleep, Duration as TokioDuration};

use crate::logger::Logger;
use crate::models::TxResult;
use crate::models::{AggregateResult, AggregateResults, DomainAggregateResult, MetadataDoc};
use crate::pipelines::{get_auto_renewal_altcoins_data, get_auto_renewal_data};
use crate::starknet_utils::check_pending_transactions;
use crate::starknetid_utils::{get_altcoin_quote, get_balances, get_renewal_price_eth};
use crate::utils::to_hex;
use crate::utils::{from_uint256, hex_to_bigdecimal, to_uint256};
use crate::{config::Config, models::AppState};

lazy_static::lazy_static! {
    static ref RENEW_TIME: FieldElement = FieldElement::from_dec_str("365").unwrap();
}

pub async fn get_domains_ready_for_renewal(
    config: &Config,
    state: &Arc<AppState>,
    logger: &Logger,
) -> Result<HashMap<FieldElement, AggregateResults>> {
    let mut results = get_auto_renewal_data(config, state).await?;
    let results_altcoins = get_auto_renewal_altcoins_data(config, state).await?;

    let mut grouped_results: HashMap<FieldElement, AggregateResults> = HashMap::new();

    if results.is_empty() && results_altcoins.is_empty() {
        grouped_results.insert(
            config.contract.erc20,
            AggregateResults {
                domains: vec![],
                renewers: vec![],
                domain_prices: vec![],
                tax_prices: vec![],
                meta_hashes: vec![],
                auto_renew_contracts: vec![],
            },
        );
        return Ok(grouped_results);
    }

    // merge all results together
    results.extend(results_altcoins.iter().cloned());

    // Fetch balances for all renewers
    let renewer_and_erc20: Vec<(String, String)> = results
        .iter()
        .map(|result| {
            (
                result.renewer_address.clone(),
                // get the erc20 address for the given auto_renew_contract
                to_hex(
                    *config
                        .altcoins_mapping
                        .get(&result.auto_renew_contract)
                        .unwrap(),
                ),
            )
        })
        .collect();

    let balances = get_balances(config, renewer_and_erc20.clone()).await;
    if balances.is_empty() {
        logger.severe("Error while fetching balances for users".to_string());
        return Err(anyhow!("Error while fetching balances"));
    }

    let dynamic_balances: Arc<Mutex<HashMap<String, HashMap<String, BigInt>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let mut balances_iter = balances.into_iter();
    for (address, erc20) in &renewer_and_erc20 {
        balances_iter.next(); // we skip the first result as its value is 2 for low and high
        let balance = from_uint256(
            balances_iter.next().expect("Expected low not found"),
            balances_iter.next().expect("Expected high not found"),
        );
        let mut outer_map = dynamic_balances.lock().unwrap();
        let inner_map = outer_map.entry(address.clone()).or_default();
        inner_map.insert(erc20.clone(), balance);
    }

    // Then process the results
    let results_stream = stream::iter(results.into_iter().enumerate());
    let processed_results = results_stream
        .then(|(i, result)| {
            let renewer_and_erc20_cloned = renewer_and_erc20.clone();
            let dynamic_balances = Arc::clone(&dynamic_balances);
            async move {
                let (address, erc20) = renewer_and_erc20_cloned.get(i).unwrap();
                let balance = dynamic_balances
                    .lock()
                    .unwrap()
                    .get(address)
                    .and_then(|erc20_balances| erc20_balances.get(erc20))
                    .cloned()
                    .expect("Balance not found for this erc20");
                let renewal_price_eth = get_renewal_price_eth(result.domain.clone());
                let renewal_price =
                    if FieldElement::from_hex_be(erc20).unwrap() == config.contract.erc20 {
                        renewal_price_eth
                    } else {
                        match get_altcoin_quote(config, erc20.to_string()).await {
                            Ok(quote) => {
                                (quote * renewal_price_eth)
                                    / BigInt::from_str("1000000000000000000").unwrap()
                            }
                            Err(e) => {
                                // in case get_altcoin_quote endpoint returns an error we panic with the error
                                logger.severe(format!(
                                    "Error while fetching quote on starknetid server: {:?}",
                                    e
                                ));
                                panic!("Error while fetching quote on starknetid server: {:?}", e)
                            }
                        }
                    };
                let output = process_aggregate_result(
                    state,
                    result.clone(),
                    logger,
                    BigDecimal::from(balance.to_owned()),
                    BigDecimal::from(renewal_price.to_owned()),
                    erc20.clone(),
                )
                .await;

                if output.is_some() {
                    let new_balance = balance - renewal_price;
                    dynamic_balances
                        .lock()
                        .unwrap()
                        .entry(erc20.to_string())
                        .or_default()
                        .insert(address.to_owned(), new_balance);
                };

                output
            }
        })
        .collect::<Vec<_>>()
        .await;

    let mut none_count = 0;
    for res_option in processed_results.into_iter() {
        if let Some(res) = res_option {
            // Fetch or initialize the AggregateResults for this key
            let entry = grouped_results
                .entry(res.auto_renew_contract)
                .or_insert_with(|| AggregateResults {
                    domains: vec![],
                    renewers: vec![],
                    domain_prices: vec![],
                    tax_prices: vec![],
                    meta_hashes: vec![],
                    auto_renew_contracts: vec![],
                });

            // Append the current result to the vectors in AggregateResults
            entry.domains.push(res.domain);
            entry.renewers.push(res.renewer_addr);
            entry.domain_prices.push(res.domain_price);
            entry.tax_prices.push(res.tax_price);
            entry.meta_hashes.push(res.meta_hash);
        } else {
            // Increment none_count if the result is None
            none_count += 1;
        }
    }
    logger.warning(format!("Domains that couldn't be renewed: {}", none_count));

    Ok(grouped_results)
}

async fn process_aggregate_result(
    state: &Arc<AppState>,
    result: DomainAggregateResult,
    _logger: &Logger,
    balance: BigDecimal,
    renewal_price: BigDecimal,
    erc20_addr: String,
) -> Option<AggregateResult> {
    // Skip the rest if auto-renewal is not enabled
    if !result.enabled || result.allowance.is_none() {
        return None;
    }

    let renewer_addr = FieldElement::from_hex_be(&result.renewer_address).unwrap();
    // map the vec of approval_values to get tha approval_value for the erc20_addr selected
    let erc20_allowance = if let Some(approval_value) = result
        .approval_values
        .iter()
        .find(|&data| data.erc20_addr == erc20_addr)
        .map(|data| data.approval_value.clone())
    {
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

    // Check user ERC20 allowance is greater or equal than final price = renew_price + tax_price
    if erc20_allowance >= final_price {
        // check user allowance is greater or equal than final price
        if allowance >= final_price {
            // check user balance is sufficiant
            if balance < final_price {
                // println!(
                //     "Domain {} cannot be renewed because {} has not enough balance ({}) for domain price({})",
                //     result.domain, result.renewer_address, balance, final_price
                // );
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
                auto_renew_contract: result.auto_renew_contract,
            })
        } else {
            // println!(
            //     "Domain {} cannot be renewed because {} has set an allowance({}) lower than final price({})",
            //     result.domain, result.renewer_address, allowance, final_price
            // );
            None
        }
    } else {
        // println!(
        //     "Domain {} cannot be renewed because {} has set an erc20_allowance ({}) lower than domain price({}) + tax({})",
        //     result.domain, result.renewer_address, erc20_allowance, renewal_price, tax_price
        // );
        None
    }
}

pub async fn renew_domains(
    config: &Config,
    account: &SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    mut aggregate_results: AggregateResults,
    auto_renew_contract: &FieldElement,
    logger: &Logger,
) -> Result<()> {
    logger.info(format!(
        "Renewing {} domains on autorenewal contract {}",
        aggregate_results.domains.len(),
        auto_renew_contract
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
            account,
            auto_renew_contract.to_owned(),
            AggregateResults {
                domains: domains_to_renew.clone(),
                renewers,
                domain_prices,
                tax_prices,
                meta_hashes,
                auto_renew_contracts: vec![],
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
    account: &SingleOwnerAccount<JsonRpcClient<HttpTransport>, LocalWallet>,
    auto_renew_contract: FieldElement,
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
            to: auto_renew_contract,
            selector: selector!("batch_renew"),
            calldata: calldata.clone(),
        }])
        .fee_estimate_multiplier(5.0f64);

    match execution.estimate_fee().await {
        Ok(_) => match execution
            .nonce(nonce)
            // harcode max fee to 10$ = 0.0028 ETH
            .max_fee(FieldElement::from(2800000000000000_u64))
            .send()
            .await
        {
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
