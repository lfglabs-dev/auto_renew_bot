use anyhow::Result;
use bson::{doc, Bson};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use starknet::core::types::FieldElement;
use std::sync::Arc;

use crate::{
    config::Config,
    models::{AppState, Domain, DomainAggregateResult},
    utils::to_hex,
};

pub async fn get_auto_renewal_data(
    config: &Config,
    state: &Arc<AppState>,
) -> Result<Vec<DomainAggregateResult>> {
    let auto_renews_collection = state.db.collection::<Domain>("auto_renew_flows");
    let min_expiry_date = Utc::now() + Duration::days(config.renewals.expiry_days);
    let erc20_addr = to_hex(config.contract.erc20);
    let auto_renew_contract = FieldElement::to_string(&config.contract.renewal);
    println!("timestamp: {}", min_expiry_date.timestamp());
    // Define aggregate pipeline
    let pipeline = vec![
        doc! { "$match": { "_cursor.to": null } },
        doc! { "$match": { "enabled": true } },
        doc! { "$lookup": {
            "from": "domains",
            "let": { "domain_name": "$domain" },
            "pipeline": [
                { "$match":
                    { "$expr":
                        { "$and": [
                            { "$eq": [ "$domain",  "$$domain_name" ] },
                            { "$eq": [ { "$ifNull": [ "$_cursor.to", null ] }, null ] },
                        ]}
                    }
                },
            ],
            "as": "domain_info",
        }},
        doc! { "$unwind": "$domain_info" },
        doc! { "$match": { "domain_info.expiry": { "$lt": Bson::Int64(min_expiry_date.timestamp()) } } },
        doc! { "$lookup": {
            "from": "auto_renew_approvals",
            "let": { "renewer_addr": "$renewer_address" },
            "pipeline": [
                { "$match":
                    { "$expr":
                        { "$and": [
                            { "$eq": [ "$renewer",  "$$renewer_addr" ] },
                            { "$eq": [ { "$ifNull": [ "$_cursor.to", null ] }, null ] },
                        ]}
                    }
                }
            ],
            "as": "approval_info",
        }},
        doc! { "$unwind": { "path": "$approval_info", "preserveNullAndEmptyArrays": true } },
        doc! { "$addFields": {
            "auto_renew_contract": auto_renew_contract,
        }},
        doc! { "$group": {
            "_id": "$domain_info.domain",
            "expiry": { "$first": "$domain_info.expiry" },
            "renewer_address": { "$first": "$renewer_address" },
            "enabled": { "$first": "$enabled" },
            "allowance": { "$first": "$allowance" },
            "last_renewal": { "$first": "$last_renewal" },
            "meta_hash": { "$first": "$meta_hash" },
            "_cursor": { "$first": "$_cursor" },
            "auto_renew_contract": { "$first": "$auto_renew_contract" },
            "approval_values": {
                "$push": {
                    "approval_value": { "$ifNull": [ "$approval_info.allowance", "0x0" ] },
                    "erc20_addr": erc20_addr
                }
            },
        }},
        doc! { "$project": {
            "_id": 0,
            "domain": "$_id",
            "expiry": 1,
            "renewer_address": 1,
            "enabled": 1,
            "allowance": 1,
            "last_renewal": 1,
            "meta_hash": 1,
            "_cursor": 1,
            "auto_renew_contract": 1,
            "approval_values": 1,
        }},
    ];

    // Execute the pipeline
    let cursor = auto_renews_collection.aggregate(pipeline, None).await?;
    // Extract the results as Vec<bson::Document>
    let bson_docs: Vec<bson::Document> = cursor.try_collect().await?;
    // Convert each bson::Document into DomainAggregateResult
    let results: Result<Vec<DomainAggregateResult>, _> = bson_docs
        .into_iter()
        .map(|doc| bson::from_bson(bson::Bson::Document(doc)))
        .collect();
    // Check if the conversion was successful
    let results = results?;

    Ok(results)
}

pub async fn get_auto_renewal_altcoins_data(
    config: &Config,
    state: &Arc<AppState>,
) -> Result<Vec<DomainAggregateResult>> {
    let auto_renews_collection = state.db.collection::<Domain>("auto_renew_flows_altcoins");
    let min_expiry_date = Utc::now() + Duration::days(config.renewals.expiry_days);

    // Define aggregate pipeline
    let pipeline = vec![
        doc! { "$match": { "_cursor.to": null } },
        doc! { "$match": { "enabled": true } },
        doc! { "$lookup": {
            "from": "domains",
            "let": { "domain_name": "$domain" },
            "pipeline": [
                { "$match":
                    { "$expr":
                        { "$and": [
                            { "$eq": [ "$domain",  "$$domain_name" ] },
                            { "$eq": [ { "$ifNull": [ "$_cursor.to", null ] }, null ] },
                        ]}
                    }
                },
            ],
            "as": "domain_info",
        }},
        doc! { "$unwind": "$domain_info" },
        doc! { "$match": { "domain_info.expiry": { "$lt": Bson::Int64(min_expiry_date.timestamp()) } } },
        doc! { "$lookup": {
            "from": "auto_renew_approvals_altcoins",
            "let": { "renewer_addr": "$renewer_address" },
            "pipeline": [
                { "$match":
                    { "$expr":
                        { "$and": [
                            { "$eq": [ "$renewer",  "$$renewer_addr" ] },
                            { "$eq": [ { "$ifNull": [ "$_cursor.to", null ] }, null ] },
                        ]}
                    }
                }
            ],
            "as": "approval_info",
        }},
        doc! { "$unwind": { "path": "$approval_info", "preserveNullAndEmptyArrays": true } },
        doc! { "$group": {
            "_id": "$domain_info.domain",
            "expiry": { "$first": "$domain_info.expiry" },
            "renewer_address": { "$first": "$renewer_address" },
            "enabled": { "$first": "$enabled" },
            "allowance": { "$first": "$allowance" },
            "last_renewal": { "$first": "$last_renewal" },
            "meta_hash": { "$first": "$meta_hash" },
            "_cursor": { "$first": "$_cursor" },
            "auto_renew_contract": { "$first": "$auto_renew_contract" },
            "approval_values": {
                "$push": {
                    "approval_value": { "$ifNull": [ "$approval_info.allowance", "0x0" ] },
                    "erc20_addr": { "$ifNull": [ "$approval_info.erc20_addr", "0x0" ] },
                }
            },
        }},
        doc! { "$project": {
            "_id": 0,
            "domain": "$_id",
            "expiry": 1,
            "renewer_address": 1,
            "enabled": 1,
            "allowance": 1,
            "last_renewal": 1,
            "meta_hash": 1,
            "_cursor": 1,
            "auto_renew_contract": 1,
            "approval_values": 1,
        }},
    ];

    // Execute the pipeline
    let cursor = auto_renews_collection.aggregate(pipeline, None).await?;
    // Extract the results as Vec<bson::Document>
    let bson_docs: Vec<bson::Document> = cursor.try_collect().await?;
    // Convert each bson::Document into DomainAggregateResult
    let results: Result<Vec<DomainAggregateResult>, _> = bson_docs
        .into_iter()
        .map(|doc| bson::from_bson(bson::Bson::Document(doc)))
        .collect();
    // Check if the conversion was successful
    let results = results?;

    Ok(results)
}
