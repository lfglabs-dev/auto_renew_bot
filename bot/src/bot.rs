use std::{str::FromStr, sync::Arc};

use anyhow::{anyhow, Context, Result};
use bson::{doc, DateTime as BsonDateTime};
use chrono::{Duration, Utc};
use futures::stream::StreamExt;
use mongodb::options::FindOptions;
use serde_json::json;
use serenity::http::Http;
use serenity::model::channel::Message;
use starknet::accounts::Account;
use starknet::{
    accounts::{Call, SingleOwnerAccount},
    core::{
        types::{BlockId, CallFunction, FieldElement},
        utils::get_selector_from_name,
    },
    macros::selector,
    providers::{Provider, SequencerGatewayProvider},
    signers::LocalWallet,
};
use url::Url;

use crate::{
    config::Config,
    models::{AppState, Approval, AutoRenewals, Domain},
};
use bigdecimal::{BigDecimal, Zero};

pub fn get_provider(config: &Config) -> SequencerGatewayProvider {
    if config.devnet_provider.is_devnet {
        SequencerGatewayProvider::new(
            Url::from_str(&config.devnet_provider.gateway).unwrap(),
            Url::from_str(&config.devnet_provider.feeder_gateway).unwrap(),
        )
    } else {
        SequencerGatewayProvider::starknet_alpha_goerli()
    }
}

pub async fn get_domains_ready_for_renewal(
    config: &Config,
    state: &Arc<AppState>,
) -> Result<(Vec<FieldElement>, Vec<FieldElement>)> {
    let domains = state.db.collection::<Domain>("domains");
    let renewals = state.db.collection::<AutoRenewals>("auto_renewals");
    let approvals = state.db.collection::<Approval>("approvals");

    let min_expiry_date = Utc::now() + Duration::days(120);
    let find_options = FindOptions::builder().build();
    let mut domain_cursor = domains
        .find(
            doc! { "expiry": { "$lt": BsonDateTime::from_millis(min_expiry_date.timestamp_millis()) } },
            find_options,
        )
        .await?;

    let mut domains_ready: Vec<FieldElement> = Vec::new();
    let mut renewer_ready: Vec<FieldElement> = Vec::new();

    while let Some(domain) = domain_cursor.next().await {
        let domain = domain?;
        let renewal = renewals
            .find_one(doc! { "domain": &domain.domain }, None)
            .await?;

        if let Some(renewal) = renewal {
            if renewal.auto_renewal_enabled {
                let renewer_addr = FieldElement::from_str(&renewal.renewer_address)?;
                // get allowance from db
                let approval = approvals
                    .find_one(doc! { "renewer": &renewal.renewer_address }, None)
                    .await?;
                let allowance = if let Some(approval) = approval {
                    BigDecimal::from_str(&approval.value)?
                } else {
                    BigDecimal::zero()
                };

                // get renew price from contract
                let provider = get_provider(&config);
                let domain_name = domain
                    .domain
                    .strip_suffix(".stark")
                    .unwrap_or(&domain.domain);
                let domain_encoded = starknet::id::encode(domain_name)
                    .map_err(|_| anyhow!("Failed to encode domain name"))
                    .context("Error occurred while encoding domain name")?;

                let call_result = provider
                    .call_contract(
                        CallFunction {
                            contract_address: config.contract.pricing,
                            entry_point_selector: selector!("compute_renew_price"),
                            calldata: vec![
                                domain_encoded,
                                FieldElement::from_dec_str("60").unwrap(),
                            ],
                        },
                        BlockId::Latest,
                    )
                    .await;

                match call_result {
                    Ok(result) => {
                        let renew_price = FieldElement::to_big_decimal(&result.result[1], 18);
                        if renew_price <= allowance {
                            domains_ready.push(domain_encoded);
                            renewer_ready.push(renewer_addr);
                        } else {
                            log_msg_and_send_to_discord(
                                &config,
                                "[Renewal]",
                                &format!(
                                "Domain {} cannot be renewed because {} has not enough allowance",
                                domain.domain, renewal.renewer_address
                            ),
                            )
                            .await;
                        }
                    }
                    Err(e) => {
                        return Err(anyhow::anyhow!(
                            "Error while fetching renew priced for domain {:?} : {:?}",
                            &domain.domain,
                            e
                        ));
                    }
                }
            }
        }
    }

    Ok((domains_ready, renewer_ready))
}

pub async fn renew_domains(
    config: &Config,
    account: &SingleOwnerAccount<SequencerGatewayProvider, LocalWallet>,
    domains: (Vec<FieldElement>, Vec<FieldElement>),
) -> Result<()> {
    let mut calldata: Vec<FieldElement> = Vec::new();
    calldata.push(FieldElement::from_dec_str(&domains.0.len().to_string()).unwrap());
    calldata.extend_from_slice(&domains.0);
    calldata.push(FieldElement::from_dec_str(&domains.1.len().to_string()).unwrap());
    calldata.extend_from_slice(&domains.1);

    let result = account
        .execute(vec![Call {
            to: config.contract.renewal,
            selector: get_selector_from_name("batch_renew").unwrap(),
            calldata,
        }])
        .send()
        .await;

    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            let error_message = format!("An error occurred while renewing domains: {}", e);
            Err(anyhow::anyhow!(error_message))
        }
    }
}

pub async fn send_message_to_discord(config: &Config, message: &str) -> Result<Message> {
    let http = Http::new(&config.discord.token);

    let message_json = json!({
        "content": message,
    });
    let message = http
        .send_message(config.discord.channel_id, &message_json)
        .await
        .context("Failed to send message to Discord channel")?;

    Ok(message)
}

pub async fn log_error_and_send_to_discord(
    config: &Config,
    error_type: &str,
    error: &anyhow::Error,
) {
    let message = format!("***{}***: {}", error_type, error);
    println!("{}", message);

    if let Err(e) = send_message_to_discord(&config, &message).await {
        println!("Failed to send error message to Discord: {:?}", e);
    }
}

pub async fn log_msg_and_send_to_discord(config: &Config, log_type: &str, msg_content: &str) {
    let message = format!("_{}_: {}", log_type, msg_content);
    println!("{}", message);

    if let Err(e) = send_message_to_discord(&config, &message).await {
        println!("Failed to send log message to Discord: {:?}", e);
    }
}

pub async fn log_domains_renewed(
    config: &Config,
    domains: (Vec<FieldElement>, Vec<FieldElement>),
) -> Result<()> {
    let message = format!(
        "Domains renewed: \n {}",
        domains
            .0
            .iter()
            .zip(domains.1.iter())
            .map(|(d, r)| format!("- `{}` by `{}`", &starknet::id::decode(*d), r))
            .collect::<Vec<String>>()
            .join(" \n")
    );
    log_msg_and_send_to_discord(&config, "[Renewal]", &message).await;
    Ok(())
}
