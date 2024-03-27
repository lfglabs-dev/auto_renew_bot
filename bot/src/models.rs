use std::collections::HashMap;

use bigdecimal::BigDecimal;
use bson::DateTime;
use mongodb::Database;
use serde::{Deserialize, Serialize};
use starknet::core::types::FieldElement;

pub struct AppState {
    pub db: Database,
    pub db_metadata: Database,
    pub states: States,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Domain {
    pub domain: String,
    pub expiry: Option<DateTime>,
    pub token_id: Option<String>,
    pub creation_date: Option<DateTime>,
    pub rev_addr: Option<String>,
    pub auto_renew_contract: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Approval {
    pub renewer: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AutoRenewals {
    pub domain: String,
    pub renewer_address: String,
    pub auto_renewal_enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Cursor {
    pub to: Option<i64>,
    pub from: Option<i64>,
}

// #[derive(Clone, Debug, Deserialize, Serialize)]
// pub struct DomainAggregateResult {
//     pub domain: String,
//     pub expiry: Option<i32>,
//     pub renewer_address: String,
//     pub enabled: bool,
//     pub approval_value: Option<String>,
//     pub allowance: Option<String>,
//     pub last_renewal: Option<i64>,
//     pub meta_hash: Option<String>,
//     pub _cursor: Cursor,
//     pub erc20_addr: String,
//     pub auto_renew_contract: FieldElement,
// }

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AutoRenewAllowance {
    pub approval_value: String,
    pub erc20_addr: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DomainAggregateResult {
    pub domain: String,
    pub expiry: Option<i32>,
    pub renewer_address: String,
    pub enabled: bool,
    pub allowance: Option<String>,
    pub last_renewal: Option<i64>,
    pub meta_hash: Option<String>,
    pub _cursor: Cursor,
    pub auto_renew_contract: FieldElement,
    pub approval_values: Vec<AutoRenewAllowance>,
}

pub struct AggregateResult {
    pub domain: FieldElement,
    pub renewer_addr: FieldElement,
    pub domain_price: BigDecimal,
    pub tax_price: BigDecimal,
    pub meta_hash: FieldElement,
    pub auto_renew_contract: FieldElement,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AggregateResults {
    pub domains: Vec<FieldElement>,
    pub renewers: Vec<FieldElement>,
    pub domain_prices: Vec<BigDecimal>,
    pub tax_prices: Vec<BigDecimal>,
    pub meta_hashes: Vec<FieldElement>,
    pub auto_renew_contracts: Vec<FieldElement>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct MetadataDoc {
    pub meta_hash: String,
    pub email: String,
    pub tax_state: String,
    pub salt: String,
}

#[derive(Deserialize, Debug)]
pub struct State {
    pub rate: f32,
    #[serde(rename = "type")]
    pub type_: String,
}

#[derive(Deserialize, Debug)]
pub struct States {
    pub states: HashMap<String, State>,
}

#[derive(Debug, Clone)]
pub struct TxResult {
    pub tx_hash: FieldElement,
    pub reverted: Option<bool>,
    pub revert_reason: Option<String>,
    pub domains_renewed: usize,
}
