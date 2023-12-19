use std::{borrow::Cow, sync::Arc};

use self::status::status_client::StatusClient;
use self::status::GetStatusRequest;
use bot::renew_domains;
use bson::doc;
use mongodb::{options::ClientOptions, Client as mongoClient};
use serde_derive::Serialize;
use starknet::{
    accounts::SingleOwnerAccount,
    providers::Provider,
    signers::{LocalWallet, SigningKey},
};
use starknet_id::decode;
use starknet_utils::create_jsonrpc_client;
use tokio::time::sleep;

pub mod status {
    tonic::include_proto!("apibara.sink.v1");
}

mod bot;
mod config;
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
        starknet::accounts::ExecutionEncoding::New,
    );

    logger.info("Started");
    let mut need_to_check_status = true;
    loop {
        if need_to_check_status {
            logger.info("Checking indexer status");
            let mut is_ready = true;
            'outer: for port in &conf.indexer_server.port {
                let indexer_client =
                    StatusClient::connect(format!("{}:{}", conf.indexer_server.server_url, port))
                        .await;
                match indexer_client {
                    Ok(mut indexer) => {
                        let request = tonic::Request::new(GetStatusRequest {});
                        match indexer.get_status(request).await {
                            Ok(response) => {
                                let res = response.into_inner();
                                if let (Some(current_block), Some(head_block)) =
                                    (res.current_block, res.head_block)
                                {
                                    if current_block < head_block {
                                        logger.info(format!(
                                                "Indexer on port {} is not up to date. Current block {} is lower than head block {}. Retrying in 5 seconds.",
                                                port, current_block, head_block
                                            ));
                                        is_ready = false;
                                        break 'outer;
                                    }
                                }
                            }
                            Err(e) => {
                                logger.severe(format!(
                                    "Unable to connect to indexer on port {}: {}",
                                    port, e
                                ));
                                is_ready = false;
                                break 'outer;
                            }
                        }
                    }
                    Err(e) => {
                        logger.severe(format!(
                            "Unable to connect to indexer on port {}: {}",
                            port, e
                        ));
                        continue;
                    }
                }
            }
            if is_ready {
                need_to_check_status = false;
                logger.info("Indexer is up to date, starting renewals");
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        } else {
            println!("[bot] Checking domains to renew");
            match bot::get_domains_ready_for_renewal(&conf, &shared_state, &logger).await {
                Ok(aggregate_results) => {
                    //println!("[bot] checking domains to renew today");
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
                                            &decode(*d),
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
