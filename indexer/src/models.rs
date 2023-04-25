use mongodb::Database;
use serde::{Deserialize, Serialize};

pub struct AppState {
    pub db: Database,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Domain {
    pub addr: Option<String>,
    pub domain: Option<String>,
    pub expiry: Option<String>,
    pub token_id: Option<String>,
    pub creation_date: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DomainRenewals {
    pub domain: String,
    pub prev_expiry: String,
    pub new_expiry: String,
    pub renewal_date: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AutoRenewals {
    pub domain: String,
    pub renewer_address: String,
    pub last_renewal_date: String,
    pub auto_renewal_enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RenewedDomains {
    pub domain: String,
    pub renewer_address: String,
    pub date: String,
    pub days: i64,
}
