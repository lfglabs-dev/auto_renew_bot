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
    pub meta_hash: String,
    pub _chain: Chain,
}

pub struct AggregateResult {
    pub domain: FieldElement,
    pub renewer_addr: FieldElement,
    pub limit_price: BigDecimal,
    pub tax_price: BigDecimal,
    pub meta_hash: FieldElement,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AggregateResults {
    pub domains: Vec<FieldElement>,
    pub renewers: Vec<FieldElement>,
    pub limit_prices: Vec<BigDecimal>,
    pub tax_prices: Vec<BigDecimal>,
    pub meta_hashes: Vec<FieldElement>,
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

pub trait Unzip5 {
    type A;
    type B;
    type C;
    type D;
    type E;

    fn unzip5(
        self,
    ) -> (
        Vec<Self::A>,
        Vec<Self::B>,
        Vec<Self::C>,
        Vec<Self::D>,
        Vec<Self::E>,
    );
}

impl<T, A, B, C, D, E> Unzip5 for T
where
    T: Iterator<Item = (A, B, C, D, E)>,
{
    type A = A;
    type B = B;
    type C = C;
    type D = D;
    type E = E;

    fn unzip5(
        self,
    ) -> (
        Vec<Self::A>,
        Vec<Self::B>,
        Vec<Self::C>,
        Vec<Self::D>,
        Vec<Self::E>,
    ) {
        let mut a = Vec::new();
        let mut b = Vec::new();
        let mut c = Vec::new();
        let mut d = Vec::new();
        let mut e = Vec::new();

        for (x, y, z, w, v) in self {
            a.push(x);
            b.push(y);
            c.push(z);
            d.push(w);
            e.push(v);
        }

        (a, b, c, d, e)
    }
}
