use std::sync::Arc;

use bot::{get_provider, renew_domains};
use bson::doc;
use mongodb::{options::ClientOptions, Client};
use starknet::{
    accounts::SingleOwnerAccount,
    core::{chain_id, types::FieldElement},
    signers::{LocalWallet, SigningKey},
};
use tokio::time::sleep;

mod bot;
mod config;
mod models;

#[tokio::main]
async fn main() {
    let conf = config::load();

    let client_options = ClientOptions::parse(&conf.database.connection_string)
        .await
        .unwrap();
    let shared_state = Arc::new(models::AppState {
        db: Client::with_options(client_options)
            .unwrap()
            .database(&conf.database.name),
    });
    if shared_state
        .db
        .run_command(doc! {"ping": 1}, None)
        .await
        .is_err()
    {
        println!("error: unable to connect to database");
        return;
    } else {
        println!("database: connected")
    }

    let provider = get_provider(&conf);
    let signer = LocalWallet::from(SigningKey::from_secret_scalar(
        FieldElement::from_hex_be(&conf.account.private_key).unwrap(),
    ));
    let address = FieldElement::from_hex_be(&conf.account.address).unwrap();
    let account = SingleOwnerAccount::new(provider, signer, address, chain_id::TESTNET);

    loop {
        let domains = bot::get_domains_ready_for_renewal(&conf, &shared_state)
            .await
            .unwrap();
        match renew_domains(&conf, &account, domains).await {
            Ok(_) => println!("domains renewed successfully"),
            Err(e) => println!("error while renewing domains: {:?}", e),
        }

        // Sleep for 24 hours
        sleep(std::time::Duration::from_secs(86400)).await;
    }
}
