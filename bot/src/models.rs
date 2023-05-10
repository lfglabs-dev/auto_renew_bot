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
