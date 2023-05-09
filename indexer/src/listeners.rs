use std::sync::Arc;

use crate::{
    config,
    models::{AppState, Approval, AutoRenewals, Domain, DomainRenewals},
};
use apibara_core::starknet::v1alpha2::FieldElement;
use bigdecimal::{num_bigint::BigUint, BigDecimal, ToPrimitive};
use chrono::{DateTime, TimeZone, Utc};
use mongodb::{
    bson::{doc, Bson, DateTime as BsonDateTime},
    options::{FindOneAndUpdateOptions, InsertOneOptions, UpdateOptions},
};
use starknet::{core::types, id::decode};

pub async fn addr_to_domain_update(
    _: &config::Config,
    state: &Arc<AppState>,
    event_data: &Vec<FieldElement>,
) {
    let str_address = FieldElement::to_hex(&event_data[0]);
    let domain_len = &event_data[1];
    if domain_len == &FieldElement::from_u64(1) {
        let domain_str = match types::FieldElement::from_bytes_be(&event_data[2].to_bytes()) {
            Ok(bytes) => decode(bytes) + ".stark",
            Err(e) => {
                println!(
                    "Error decoding domain bytes: {:?} for data: {:?}",
                    e, event_data[2]
                );
                return;
            }
        };

        let domain_collection = state.db.collection::<Domain>("domains");
        let (filter, update) = if domain_str.is_empty() {
            (
                doc! {"rev_addr": &str_address, "_chain.valid_to": Bson::Null},
                doc! {"$unset": {"rev_addr": Bson::Null}},
            )
        } else {
            (
                doc! {"domain": &domain_str, "_chain.valid_to": Bson::Null},
                doc! {"$set": {"rev_addr": &str_address}},
            )
        };

        match domain_collection
            .find_one_and_update(
                filter,
                update,
                Some(FindOneAndUpdateOptions::builder().build()),
            )
            .await
        {
            Ok(_) => {
                println!("- [addr2domain] {:?} -> {:?}", str_address, domain_str);
            }
            Err(e) => {
                println!(
                    "Error while saving into db addr2domain: {:?} for domain_str = {:?} & address = {:?}",
                    e, domain_str, str_address
                );
            }
        }
    }
}

pub async fn domain_to_addr_update(
    _: &config::Config,
    state: &Arc<AppState>,
    event_data: &Vec<FieldElement>,
) {
    let domain_len = &event_data[0];
    if domain_len != &FieldElement::from_u64(1) {
        return;
    }
    let domain_str = match types::FieldElement::from_bytes_be(&event_data[1].to_bytes()) {
        Ok(bytes) => decode(bytes) + ".stark",
        Err(e) => {
            println!("Error decoding domain bytes: {:?}", e);
            return;
        }
    };
    let str_address = FieldElement::to_hex(&event_data[2]);

    if !domain_str.is_empty() {
        state
            .db
            .collection::<Domain>("domains")
            .find_one_and_update(
                doc! {"rev_addr": &domain_str, "_chain.valid_to": Bson::Null},
                doc! {"$set": {"addr": &str_address}},
                None,
            )
            .await
            .map(|_| {})
            .unwrap_or_else(|e| {
                println!("Error while saving into db domain2addr: {:?}", e);
            });
    } else {
        state
            .db
            .collection::<Domain>("domains")
            .find_one_and_update(
                doc! {"domain": &domain_str, "_chain.valid_to": Bson::Null},
                doc! {"$unset": {"addr": Bson::Null}},
                None,
            )
            .await
            .map(|_| {})
            .unwrap_or_else(|e| {
                println!("Error while saving into db domain2addr: {:?}", e);
            });
    }

    println!("- [domain2addr] {:?} -> {:?}", domain_str, str_address);
}

pub async fn on_starknet_id_update(
    _: &config::Config,
    state: &Arc<AppState>,
    event_data: &Vec<FieldElement>,
    block_timestamp: DateTime<Utc>,
) {
    let domain_len = &event_data[0];
    if domain_len != &FieldElement::from_u64(1) {
        return;
    }
    let domain_str =
        decode(types::FieldElement::from_bytes_be(&event_data[1].to_bytes()).unwrap()) + ".stark";
    let owner = BigUint::from_bytes_be(&event_data[2].to_bytes());

    let expiry: i64 = BigUint::from_bytes_be(&event_data[3].to_bytes())
        .to_u64()
        .unwrap()
        .try_into()
        .unwrap();
    let expiry_date = Utc.timestamp_opt(expiry, 0).single().unwrap();

    let filter = doc! {"domain": &domain_str, "_chain.valid_to": Bson::Null };
    let update = doc! {"$set": {"domain": &domain_str, "expiry": expiry_date.to_string(), "token_id": owner.to_string()}};
    let options = FindOneAndUpdateOptions::builder()
        .return_document(mongodb::options::ReturnDocument::After)
        .build();
    let existing = state
        .db
        .collection::<Domain>("domains")
        .find_one_and_update(filter, update, options)
        .await;

    match existing {
        Ok(Some(existing)) => {
            if let Some(ref db_expiry) = existing.expiry {
                let existing_expiry = db_expiry.timestamp_millis();
                state
                    .db
                    .collection::<DomainRenewals>("domains_renewals")
                    .insert_one(
                        DomainRenewals {
                            domain: domain_str.clone(),
                            prev_expiry: db_expiry.to_owned(),
                            new_expiry: expiry.to_string(),
                            renewal_date: BsonDateTime::from_millis(
                                block_timestamp.timestamp_millis(),
                            ),
                        },
                        None,
                    )
                    .await
                    .map(|_| {
                        println!(
                            "- [renewed] domain: {:?} id: {:?} time: {:?} days",
                            domain_str,
                            owner,
                            (expiry - existing_expiry) / 86400
                        );
                    })
                    .unwrap_or_else(|e| {
                        println!("Error while saving into db renewed domain: {:?}", e);
                    });
            } else {
                println!("Domain field is None");
            }
        }
        Ok(None) => {
            let collection = state.db.collection("domains");
            let document = doc! {
                "domain": domain_str.clone(),
                "expiry": BsonDateTime::from_millis(expiry_date.timestamp_millis()),
                "token_id": owner.to_string(),
                "creation_date": BsonDateTime::from_millis(block_timestamp.timestamp_millis()),
            };
            let options = InsertOneOptions::builder().build();
            collection
                .insert_one(document, options)
                .await
                .map(|_| {
                    println!("- [purchased] domain: {:?} id: {:?}", domain_str, owner);
                })
                .unwrap_or_else(|e| {
                    println!("Error while saving into db purchased domain: {:?}", e);
                });
        }
        Err(e) => {
            println!("Error on_starknet_id_update: {:?}", e);
        }
    }
}

pub async fn domain_transfer(
    _: &config::Config,
    state: &Arc<AppState>,
    event_data: &Vec<FieldElement>,
) {
    let domain_len = &event_data[0];
    if domain_len != &FieldElement::from_u64(1) {
        return;
    }
    let mut domain_str =
        decode(types::FieldElement::from_bytes_be(&event_data[1].to_bytes()).unwrap());
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
        state
            .db
            .collection::<Domain>("domains")
            .find_one_and_update(query, update, None)
            .await
            .map(|_| {})
            .unwrap_or_else(|e| {
                println!("Error while saving into db domain_transfer: {:?}", e);
            });
    } else {
        let collection = state.db.collection("domains");
        let document = doc! {
            "domain": &domain_str,
            "addr": "0",
            "expiry": Bson::Null,
            "token_id": prev_owner.to_string(),
        };
        let options = InsertOneOptions::builder().build();
        collection
            .insert_one(document, options)
            .await
            .map(|_| {})
            .unwrap_or_else(|e| {
                println!("Error while saving into db domain_transfer: {:?}", e);
            });
    }

    println!(
        "domain transfer: {:?} {:?} -> {:?}",
        domain_str, prev_owner, new_owner
    );
}

pub async fn toggled_renewal(
    _: &config::Config,
    state: &Arc<AppState>,
    event_data: &Vec<FieldElement>,
) {
    let domain = match types::FieldElement::from_bytes_be(&event_data[0].to_bytes()) {
        Ok(bytes) => decode(bytes) + ".stark",
        Err(e) => {
            println!("Error decoding domain bytes: {:?}", e);
            return;
        }
    };
    let renewer_address = FieldElement::to_hex(&event_data[1]);
    let value = BigUint::from_bytes_be(&event_data[2].to_bytes())
        .to_i64()
        .unwrap();
    let auto_renewal_enabled = value != 0;

    let collection = state.db.collection::<AutoRenewals>("auto_renewals");
    let filter = doc! {
        "domain": &domain,
        "renewer_address": &renewer_address,
    };
    let update = doc! {
        "$set": {
            "auto_renewal_enabled": auto_renewal_enabled
        }
    };
    let options = UpdateOptions::builder().upsert(true).build();
    match collection.update_one(filter, update, options).await {
        Ok(_) => {
            println!(
                "- [toggled_renewal] domain: {:?} renewer: {:?} value: {:?}",
                domain, renewer_address, auto_renewal_enabled
            );
        }
        Err(e) => {
            println!("Error while saving into db renewed domain: {:?} for domain: {:?} and renewer: {:?}", e, domain, renewer_address);
        }
    }
}

pub async fn approval_update(
    config: &config::Config,
    state: &Arc<AppState>,
    event_data: &Vec<FieldElement>,
) {
    let spender = FieldElement::to_hex(&event_data[1]);
    let naming_contract = FieldElement::to_hex(&config.contract.naming);
    if spender == naming_contract {
        let renewer = FieldElement::to_hex(&event_data[0]);
        let allowance =
            BigDecimal::new(BigUint::from_bytes_be(&event_data[2].to_bytes()).into(), 18)
                .to_string();
        let approval_collection = state.db.collection::<Approval>("approvals");
        let filter = doc! {
            "renewer": &renewer,
        };
        let update = doc! {
            "$set": {
                "renewer": &renewer,
                "value": &allowance,
            }
        };
        let options = UpdateOptions::builder().upsert(true).build();
        match approval_collection
            .update_one(filter, update, options)
            .await
        {
            Ok(_) => {
                println!(
                    "- [approval_update] renewer: {:?} -> value : {:?}",
                    renewer, allowance
                );
            }
            Err(e) => {
                eprintln!(
                    "Error while saving into approval event into db : {:?} for renewer {:?} and value {:?}",
                    e, renewer, allowance
                );
            }
        }
    }
}
