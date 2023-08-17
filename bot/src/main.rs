use std::sync::Arc;

use bot::{get_chainid, get_provider, renew_domains};
use bson::doc;
use discord::{log_domains_renewed, log_error_and_send_to_discord, log_msg_and_send_to_discord};
use mongodb::{options::ClientOptions, Client as mongoClient};
use starknet::{
    accounts::SingleOwnerAccount,
    signers::{LocalWallet, SigningKey},
};
use tokio::time::sleep;

mod bot;
mod config;
mod discord;
mod indexer_status;
mod models;

#[tokio::main]
async fn main() {
    let conf = config::load();

    let client_options = ClientOptions::parse(&conf.database.connection_string)
        .await
        .unwrap();
    let shared_state = Arc::new(models::AppState {
        db: mongoClient::with_options(client_options)
            .unwrap()
            .database(&conf.database.name),
    });
    if shared_state
        .db
        .run_command(doc! {"ping": 1}, None)
        .await
        .is_err()
    {
        log_error_and_send_to_discord(
            &conf,
            "[bot][error]",
            &anyhow::anyhow!("Unable to connect to database"),
        )
        .await;
        return;
    } else {
        log_msg_and_send_to_discord(&conf, "[bot]", "connected to database").await;
    }

    let provider = get_provider(&conf);
    let chainid = get_chainid(&conf);
    let signer = LocalWallet::from(SigningKey::from_secret_scalar(conf.account.private_key));
    let account = SingleOwnerAccount::new(provider, signer, conf.account.address, chainid);
    println!("[bot] started");
    let mut need_to_check_status = true;
    loop {
        if need_to_check_status {
            println!("[bot] Checking indexer status");
            match indexer_status::get_status_from_endpoint(&conf).await {
                Ok(block) => {
                    println!("Block: {}", block);
                    match indexer_status::check_block_status(&conf, block).await {
                        Ok(status) => {
                            if status {
                                need_to_check_status = false;
                                log_msg_and_send_to_discord(
                                    &conf,
                                    "[bot][renewals]",
                                    "Indexer is up to date, starting renewals",
                                )
                                .await
                            } else {
                                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                                continue;
                            }
                        }
                        Err(error) => {
                            println!(
                                "Error while checking block status: {}, retrying in 5 seconds",
                                error
                            );
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
            match bot::get_domains_ready_for_renewal(&conf, &shared_state).await {
                Ok(domains) => {
                    println!("[bot] checking domains to renew today");
                    if !domains.0.is_empty() && !domains.1.is_empty() && !domains.2.is_empty() {
                        match renew_domains(&conf, &account, domains.clone()).await {
                            Ok(_) => {
                                match log_domains_renewed(&conf, domains).await {
                                    Ok(_) => {log_msg_and_send_to_discord(
                                        &conf,
                                        "[bot][renewals]",
                                        "All domains renewed successfully",
                                    )
                                    .await}
                                    Err(error) => log_error_and_send_to_discord(&conf,"[bot][error] An error occurred while logging domains renewed into Discord",  &error).await
                                };
                            }
                            Err(e) => {
                                log_error_and_send_to_discord(
                                    &conf,
                                    "[bot][error] An error occurred while renewing domains",
                                    &e,
                                )
                                .await;
                                if e.to_string().contains("request rate limited") {
                                    continue;
                                } else {
                                    break;
                                }
                            }
                        }
                    } else {
                        log_msg_and_send_to_discord(
                            &conf,
                            "[bot][renewals]",
                            "No domains to renew today",
                        )
                        .await;
                    }
                }
                Err(e) => {
                    log_error_and_send_to_discord(
                        &conf,
                        "[bot][error] An error occurred while getting domains ready for renewal",
                        &e,
                    )
                    .await;
                }
            }
            // Sleep for 24 hours
            sleep(std::time::Duration::from_secs(conf.renewals.delay)).await;
        }
    }
}
