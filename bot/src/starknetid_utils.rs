use anyhow::{anyhow, Result};
use bigdecimal::{num_bigint::BigInt, FromPrimitive};
use serde::Deserialize;
use starknet::{
    core::types::{BlockId, BlockTag, FieldElement, FunctionCall},
    macros::selector,
    providers::Provider,
};
use std::str::FromStr;

use crate::{config::Config, starknet_utils::create_jsonrpc_client};

lazy_static::lazy_static! {
    static ref PRICE_DOMAIN_LEN_1: BigInt = BigInt::from_u128(1068493150684932 * 365).unwrap();
    static ref PRICE_DOMAIN_LEN_2: BigInt = BigInt::from_u128(657534246575343 * 365).unwrap();
    static ref PRICE_DOMAIN_LEN_3: BigInt = BigInt::from_u128(410958904109590 * 365).unwrap();
    static ref PRICE_DOMAIN_LEN_4: BigInt = BigInt::from_u128(232876712328767 * 365).unwrap();
    static ref PRICE_DOMAIN: BigInt = BigInt::from_u128(24657534246575 * 365).unwrap();
}

pub fn get_renewal_price_eth(domain: String) -> BigInt {
    let domain_name = domain
        .strip_suffix(".stark")
        .ok_or_else(|| anyhow::anyhow!("Invalid domain name: {:?}", domain))
        .unwrap();
    let domain_len = domain_name.len();
    match domain_len {
        1 => PRICE_DOMAIN_LEN_1.clone(),
        2 => PRICE_DOMAIN_LEN_2.clone(),
        3 => PRICE_DOMAIN_LEN_3.clone(),
        4 => PRICE_DOMAIN_LEN_4.clone(),
        _ => PRICE_DOMAIN.clone(),
    }
}

#[derive(Deserialize, Debug)]
pub struct QuoteQueryResult {
    quote: String,
    max_quote_validity: u64,
}

pub async fn get_altcoin_quote(config: &Config, erc20: String) -> Result<BigInt> {
    // Get quote from starknetid api
    let url = format!(
        "{}/get_altcoin_quote?erc20_addr={}",
        config.server.starknetid_api, erc20
    );
    let client = reqwest::Client::new();
    match client.get(&url).send().await {
        Ok(response) => match response.text().await {
            Ok(text) => match serde_json::from_str::<QuoteQueryResult>(&text) {
                Ok(results) => {
                    Ok(BigInt::from_str(&results.quote).unwrap())
                }
                Err(err) => Err(anyhow!("Error parsing response: {:?}", err)),
            },
            Err(err) => Err(anyhow!("Error fetching response: {:?}", err)),
        },
        Err(err) => Err(anyhow!("Error fetching quote: {:?}", err)),
    }
}

pub async fn get_balances(
    config: &Config,
    mut renewer_and_erc20: Vec<(String, String)>,
) -> Vec<FieldElement> {
    let mut balances: Vec<FieldElement> = vec![];

    while !renewer_and_erc20.is_empty() {
        let size = renewer_and_erc20.len().min(2500);
        let batch = renewer_and_erc20.drain(0..size).collect();
        let batch_balances = fetch_users_balances(config, batch).await;
        // we skip the first 2 elements, the first one is the index of the call, the 2nd the length of the results
        balances.extend(batch_balances.into_iter().skip(2));
    }

    balances
}

pub async fn fetch_users_balances(
    config: &Config,
    renewers_and_erc20: Vec<(String, String)>,
) -> Vec<FieldElement> {
    let mut calls: Vec<FieldElement> = vec![FieldElement::from(renewers_and_erc20.len())];
    for (renewer, erc20) in &renewers_and_erc20 {
        calls.push(FieldElement::from_hex_be(erc20).unwrap());
        calls.push(selector!("balanceOf"));
        calls.push(FieldElement::ONE);
        calls.push(FieldElement::from_hex_be(renewer).unwrap());
    }

    let provider = create_jsonrpc_client(config);
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
