use mongodb::{bson::DateTime, Database};
use serde::{Deserialize, Serialize};

pub struct AppState {
    pub db: Database,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Domain {
    pub addr: Option<String>,
    pub domain: Option<String>,
    pub expiry: Option<DateTime>,
    pub token_id: Option<String>,
    pub creation_date: Option<DateTime>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DomainRenewals {
    pub domain: String,
    pub prev_expiry: DateTime,
    pub new_expiry: String,
    pub renewal_date: DateTime,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AutoRenewals {
    pub domain: String,
    pub renewer_address: String,
    pub last_renewal_date: DateTime,
    pub auto_renewal_enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RenewedDomains {
    pub domain: String,
    pub renewer_address: String,
    pub date: DateTime,
    pub days: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Approval {
    pub renewer: String,
    pub value: String,
}
