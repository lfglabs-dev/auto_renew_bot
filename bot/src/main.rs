use std::{borrow::Cow, sync::Arc};

use bot::renew_domains;
use bson::doc;
use mongodb::{options::ClientOptions, Client as mongoClient};
use serde_derive::Serialize;
use starknet::{
    accounts::SingleOwnerAccount,
    providers::Provider,
    signers::{LocalWallet, SigningKey},
};
use starknet_utils::create_jsonrpc_client;
use tokio::time::sleep;

mod bot;
mod config;
mod indexer_status;
mod logger;
mod models;
mod sales_tax;
mod starknet_utils;
mod utils;

#[derive(Serialize)]
struct LogData<'a> {
    token: &'a str,
    log: LogPayload<'a>,
}

#[derive(Serialize)]
struct LogPayload<'a> {
    app_id: &'a str,
    r#type: &'a str,
    message: Cow<'a, str>,
    timestamp: i64,
}

#[tokio::main]
async fn main() {
    let conf = config::load();
    let logger = logger::Logger::new(&conf.watchtower);

    let states = sales_tax::load_sales_tax(&logger).await;
    if states.states.is_empty() {
        return;
    }

    let client_options = ClientOptions::parse(&conf.database.connection_string)
        .await
        .unwrap();
    let client_options_metadata = ClientOptions::parse(&conf.database.connection_string_metadata)
        .await
        .unwrap();
    let shared_state = Arc::new(models::AppState {
        db: mongoClient::with_options(client_options)
            .unwrap()
            .database(&conf.database.name),
        db_metadata: mongoClient::with_options(client_options_metadata)
            .unwrap()
            .database(&conf.database.metadata_name),
        states,
    });
    if shared_state
        .db
        .run_command(doc! {"ping": 1}, None)
        .await
        .is_err()
    {
        logger.async_severe("Unable to connect to database").await;
        return;
    } else {
        logger.info("Connected to database");
    }

    if shared_state
        .db_metadata
        .run_command(doc! {"ping": 1}, None)
        .await
        .is_err()
    {
        logger
            .async_severe("Unable to connect to metadata database")
            .await;
        return;
    } else {
        logger.info("Connected to metadata database");
    }

    let provider = create_jsonrpc_client(&conf);
    let chainid = provider.chain_id().await.unwrap();
    let signer = LocalWallet::from(SigningKey::from_secret_scalar(conf.account.private_key));
    let account = SingleOwnerAccount::new(
        provider,
        signer,
        conf.account.address,
        chainid,
        starknet::accounts::ExecutionEncoding::Legacy,
    );

    logger.info("Started");
    // todo: passed to false to now, until we have a way to check if the indexer is up to date    
    let mut need_to_check_status = true;
    loop {
        if need_to_check_status {
            logger.info("Checking indexer status");
            match indexer_status::get_status_from_endpoint(&conf).await {
                Ok(block) => {
                    println!("Block: {}", block);
                    match indexer_status::check_block_status(&conf, block).await {
                        Ok(status) => {
                            if status {
                                need_to_check_status = false;
                                logger.info("Indexer is up to date, starting renewals")
                            } else {
                                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                                continue;
                            }
                        }
                        Err(error) => {
                            logger.severe(format!(
                                "Error while checking block status: {}, retrying in 5 seconds",
                                error
                            ));
                            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        }
                    }
                }
                Err(error) => {
                    println!(
                        "Error getting indexer status, retrying in 5 seconds: {}",
                        error
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        } else {
            println!("[bot] Checking domains to renew");
            match bot::get_domains_ready_for_renewal(&conf, &shared_state, &logger).await {
                Ok(aggregate_results) => {
                    println!("[bot] checking domains to renew today");
                    if !aggregate_results.domains.is_empty() {
                        match renew_domains(&conf, &account, aggregate_results.clone(), &logger)
                            .await
                        {
                            Ok(_) => {
                                aggregate_results
                                    .domains
                                    .iter()
                                    .zip(aggregate_results.renewers.iter())
                                    .for_each(|(d, r)| {
                                        logger.info(format!(
                                            "- `Renewal: {}` by `{:#x}`",
                                            &starknet::id::decode(*d),
                                            r
                                        ))
                                    });
                            }
                            Err(e) => {
                                logger.severe(format!("Unable to renew domains: {}", e));
                                if e.to_string().contains("request rate limited") {
                                    continue;
                                } else {
                                    break;
                                }
                            }
                        }
                    } else {
                        logger.info("No domains to renew today");
                    }
                }
                Err(e) => {
                    logger.severe(format!(
                        "Unable to retrieve domains ready for renewal: {}",
                        e
                    ));
                }
            }
            // Sleep for 24 hours
            sleep(std::time::Duration::from_secs(conf.renewals.delay)).await;
        }
    }
}
