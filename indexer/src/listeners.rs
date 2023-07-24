use std::sync::Arc;

use crate::{
    apibara_utils, config,
    models::AppState,
};
use anyhow::{anyhow, Result};
use apibara_core::starknet::v1alpha2::FieldElement;
use bigdecimal::{num_bigint::BigUint, BigDecimal, ToPrimitive};
use chrono::{DateTime, TimeZone, Utc};
use mongodb::bson::{doc, Bson, DateTime as BsonDateTime};
use starknet::{core::types, id::decode};

pub async fn addr_to_domain_update(
    _: &config::Config,
    state: &Arc<AppState>,
    event_data: &Vec<FieldElement>,
    order_key: u64,
) -> Result<()> {
    let str_address = BigUint::from_bytes_be(&event_data[0].to_bytes()).to_string();
    let domain_len = &event_data[1];
    if domain_len == &FieldElement::from_u64(1) {
        let domain_str = types::FieldElement::from_bytes_be(&event_data[2].to_bytes())
            .map_err(|_| anyhow!("Error decoding domain bytes for data: {:?}", event_data[2]))?;
        let domain_str = decode(domain_str) + ".stark";

        apibara_utils::find_one_and_update(
            state,
            "domains",
            doc! {"domain": &domain_str, "_chain.valid_to": Bson::Null},
            doc! {"$unset": {"rev_addr": Bson::Null}},
            order_key,
        )
        .await
        .map_err(|e| anyhow!("Error while saving into db addr2domain: {:?} for domain_str = {:?} & address = {:?}", e, domain_str, str_address))?;

        if !domain_str.is_empty() {
            apibara_utils::find_one_and_update(
                state,
                "domains",
                doc! {"domain": &domain_str, "_chain.valid_to": Bson::Null},
                doc! {"$set": {"rev_addr": &domain_str}},
                order_key,
            )
            .await.map_err(|e| anyhow!("Error while saving into db addr2domain: {:?} for domain_str = {:?} & address = {:?}", e, domain_str, str_address))?;
        };

        println!("- [addr2domain] {:?} -> {:?}", str_address, domain_str);
    }
    Ok(())
}

pub async fn domain_to_addr_update(
    _: &config::Config,
    state: &Arc<AppState>,
    event_data: &Vec<FieldElement>,
    order_key: u64,
) -> Result<()> {
    let domain_len = &event_data[0];
    if domain_len != &FieldElement::from_u64(1) {
        return Ok(());
    }
    let domain_str = types::FieldElement::from_bytes_be(&event_data[1].to_bytes())
        .map_err(|_| anyhow!("Error decoding domain bytes"))?;
    let domain_str = decode(domain_str) + ".stark";
    let str_address = BigUint::from_bytes_be(&event_data[2].to_bytes()).to_string();

    if !domain_str.is_empty() {
        apibara_utils::find_one_and_update(
            state,
            "domains",
            doc! {"rev_addr": &domain_str, "_chain.valid_to": Bson::Null},
            doc! {"$set": {"addr": &str_address}},
            order_key,
        )
        .await
        .map_err(|e| anyhow!("Error while saving into db domain2addr: {:?}", e))?;
    } else {
        apibara_utils::find_one_and_update(
            state,
            "domains",
            doc! {"domain": &domain_str, "_chain.valid_to": Bson::Null},
            doc! {"$unset": {"addr": Bson::Null}},
            order_key,
        )
        .await
        .map_err(|e| anyhow!("Error while saving into db domain2addr: {:?}", e))?;
    }

    println!("- [domain2addr] {:?} -> {:?}", domain_str, str_address);
    Ok(())
}

pub async fn on_starknet_id_update(
    _: &config::Config,
    state: &Arc<AppState>,
    event_data: &Vec<FieldElement>,
    block_timestamp: DateTime<Utc>,
    order_key: u64,
) -> Result<()> {
    let domain_len = &event_data[0];
    if domain_len != &FieldElement::from_u64(1) {
        return Ok(());
    }
    let domain_str = types::FieldElement::from_bytes_be(&event_data[1].to_bytes())
        .map_err(|_| anyhow::anyhow!("Error decoding domain bytes"))?;
    let domain_str = decode(domain_str) + ".stark";
    let owner = BigUint::from_bytes_be(&event_data[2].to_bytes());

    let expiry: i64 = BigUint::from_bytes_be(&event_data[3].to_bytes())
        .to_u64()
        .ok_or_else(|| anyhow::anyhow!("Failed to convert to u64"))?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Failed to convert u64 to i64"))?;

    let expiry_date = Utc
        .timestamp_opt(expiry, 0)
        .single()
        .ok_or_else(|| anyhow::anyhow!("Failed to create timestamp"))?;

    match apibara_utils::find_one_and_update(
        state, 
        "domains", 
        doc! {"domain": &domain_str, "_chain.valid_to": Bson::Null }, 
        doc! {"$set": {"domain": &domain_str, "expiry": expiry_date.to_string(), "token_id": owner.to_string()}}, 
        order_key
    ).await {
        Ok(existing) => {
            match existing {
                Some(existing) => {
                    let prev_expiry = existing.get("expiry");
                    apibara_utils::insert_one(
                        state, 
                        "domains", 
                        doc! {"domain": domain_str.clone(), 
                        "prev_expiry": prev_expiry, 
                        "new_expiry":  expiry.to_string(), 
                        "renewal_date": BsonDateTime::from_millis(block_timestamp.timestamp_millis())}, order_key
                    )
                    .await
                    .map(|_| {
                        println!("- [renewed] domain: {:?} id: {:?}, new_expiry: {:?}", domain_str, owner, expiry.to_string());
                    })
                    .map_err(|e| {
                        anyhow::anyhow!("Error while saving into db renewed domain: {:?}", e)
                    })?;
                }
                None => {
                    println!("existing = None");
                    let document = doc! {
                        "domain": domain_str.clone(),
                        "expiry": BsonDateTime::from_millis(expiry_date.timestamp_millis()),
                        "token_id": owner.to_string(), 
                        "creation_date": BsonDateTime::from_millis(block_timestamp.timestamp_millis()),
                    };
                    apibara_utils::insert_one(state, "domains", document, order_key)
                    .await
                    .map(|_| {
                        println!("- [purchased] domain: {:?} id: {:?}", domain_str, owner);
                    })
                    .map_err(|e| {
                        anyhow::anyhow!("Error while saving into db purchased domain: {:?}", e)
                    })?;
                }
            }
        }
        Err(e) => {
            return Err(anyhow::anyhow!(
                "Error while saving into db addr2domain: {:?} for domain_str = {:?}",
                e, domain_str
            ));
        }
    }
    Ok(())
}

pub async fn domain_transfer(
    _: &config::Config,
    state: &Arc<AppState>,
    event_data: &Vec<FieldElement>,
    order_key: u64
) -> Result<()> {
    let domain_len = &event_data[0];
    if domain_len != &FieldElement::from_u64(1) {
        return Ok(());
    }
    let domain_str = types::FieldElement::from_bytes_be(&event_data[1].to_bytes())
        .map_err(|_| anyhow!("Error decoding domain bytes"))?;
    let mut domain_str = decode(domain_str);
    domain_str += ".stark";

    let prev_owner = BigUint::from_bytes_be(&event_data[2].to_bytes());
    let new_owner = BigUint::from_bytes_be(&event_data[3].to_bytes());

    if prev_owner.to_i64() != 0.into() {
        let query = doc! {
            "domain": &domain_str,
            "token_id": prev_owner.to_string(),
            "_chain.valid_to": Bson::Null,
        };
        let update = doc! {
            "$set": {"token_id": new_owner.to_string()}
        };
        apibara_utils::find_one_and_update(state, "domains", query, update, order_key)
            .await
            .map_err(|e| anyhow!("Error while saving into db domain_transfer: {:?}", e))?;
    } else {
        let document = doc! {
            "domain": &domain_str,
            "addr": "0",
            "expiry": Bson::Null,
            "token_id": prev_owner.to_string(),
        };
        apibara_utils::insert_one(state, "domains", document, order_key)
            .await
            .map_err(|e| anyhow!("Error while saving into db domain_transfer: {:?}", e))?;
    }

    println!(
        "domain transfer: {:?} {:?} -> {:?}",
        domain_str, prev_owner, new_owner
    );
    Ok(())
}

pub async fn toggled_renewal(
    _: &config::Config,
    state: &Arc<AppState>,
    event_data: &Vec<FieldElement>,
    order_key: u64
) -> Result<()> {
    let domain = types::FieldElement::from_bytes_be(&event_data[0].to_bytes())
        .map_err(|_| anyhow!("Error decoding domain bytes"))?;
    let domain = decode(domain) + ".stark";

    let renewer_address = BigUint::from_bytes_be(&event_data[1].to_bytes());
    let value = BigUint::from_bytes_be(&event_data[2].to_bytes())
        .to_i64()
        .ok_or_else(|| anyhow!("Failed to convert to i64"))?;
    let auto_renewal_enabled = value != 0;

    let filter = doc! {
        "domain": &domain,
        "renewer_address": &renewer_address.to_string(),
    };
    let update = doc! {
        "$set": {
            "auto_renewal_enabled": auto_renewal_enabled
        }
    };
    apibara_utils::find_one_and_update(state, "auto_renewals", filter, update, order_key)
    .await
    .map(|_| {
        println!("- [toggled_renewal] domain: {:?} renewer: {:?} value: {:?}",
        domain, renewer_address, auto_renewal_enabled);
    })
    .map_err(|e| anyhow!("Error while saving into db renewed domain: {:?} for domain: {:?} and renewer: {:?}", e, domain, renewer_address))?;
    Ok(())
}

pub async fn approval_update(
    config: &config::Config,
    state: &Arc<AppState>,
    event_data: &Vec<FieldElement>,
    order_key: u64,
) -> Result<()> {
    let spender = FieldElement::to_hex(&event_data[1]);
    let renewal_contract = FieldElement::to_hex(&config.contract.renewal);
    if spender == renewal_contract {
        let renewer = BigUint::from_bytes_be(&event_data[0].to_bytes());
        let allowance =
            BigDecimal::new(BigUint::from_bytes_be(&event_data[2].to_bytes()).into(), 18)
                .to_string();

        let filter = doc! {
            "renewer": &renewer.to_string(),
        };
        let update = doc! {
            "$set": {
                "renewer": &renewer.to_string(),
                "value": &allowance,
            },
        };
        apibara_utils::find_one_and_update(state, "approvals", filter, update, order_key).await
        .map(|_| {
            println!(
                "- [approval_update] renewer: {:?} -> value : {:?}",
                renewer, allowance
            );
        })
        .map_err(|e| anyhow!("Error while saving into approval event into db : {:?} for renewer {:?} and value {:?}",
        e, renewer, allowance))?;
    }
    Ok(())
}
