use bson::DateTime;
use mongodb::Database;
use serde::{Deserialize, Serialize};

pub struct AppState {
    pub db: Database,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Domain {
    pub domain: String,
    pub expiry: Option<DateTime>,
    pub token_id: Option<String>,
    pub creation_date: Option<DateTime>,
    pub rev_addr: Option<String>,
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
pub struct Chain {
    pub valid_to: Option<u32>,
    pub valid_from: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DomainAggregateResult {
    pub domain: String,
    pub expiry: Option<i32>,
    pub renewer_address: String,
    pub auto_renewal_enabled: bool,
    pub approval_value: String,
    pub limit_price: String,
    pub last_renewal: String,
    pub _chain: Chain,
}
