use std::sync::Arc;

use bot::{get_provider, renew_domains};
use bson::doc;
use discord::{log_domains_renewed, log_error_and_send_to_discord, log_msg_and_send_to_discord};
use mongodb::{options::ClientOptions, Client as mongoClient};
use starknet::{
    accounts::{Account, Call, SingleOwnerAccount},
    core::{chain_id, types::FieldElement},
    macros::selector,
    signers::{LocalWallet, SigningKey},
};
use tokio::time::sleep;

mod bot;
mod config;
mod discord;
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
    let signer = LocalWallet::from(SigningKey::from_secret_scalar(conf.account.private_key));
    let account =
        SingleOwnerAccount::new(provider, signer, conf.account.address, chain_id::TESTNET);
    println!("[bot] started");
    loop {
        match bot::get_domains_ready_for_renewal(&conf, &shared_state).await {
            Ok(domains) => {
                println!("[indexer] checking domains to renew today");
                if !domains.0.is_empty() && !domains.1.is_empty() {
                    // discord logs added
                    match log_domains_renewed(&conf, domains.clone()).await {
                        Ok(_) => {log_msg_and_send_to_discord(
                            &conf,
                            "[bot][renewals]",
                            "Domains to renew :",
                        )
                        .await}
                        Err(error) => log_error_and_send_to_discord(&conf,"[bot][error] An error occurred while logging domains to be renewed into Discord",  &error).await
                    };
                    // end discord logs
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
                            break;
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
