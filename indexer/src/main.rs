extern crate lazy_static;
use std::{
    net::Ipv4Addr,
    sync::{Arc, Mutex},
};

use apibara_core::starknet::v1alpha2::{Block, Filter};
use apibara_sdk::{ClientBuilder, Uri};
use discord::{log_error_and_send_to_discord, log_msg_and_send_to_discord};
use mongodb::{bson::doc, options::ClientOptions, Client};
use processing::ProcessingError;
mod apibara;
mod apibara_utils;
mod config;
mod discord;
mod endpoints;
mod listeners;
mod models;
mod processing;

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

    let shared_order_key: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None)); // assuming the order_key is Option<String>

    if shared_state
        .db
        .run_command(doc! {"ping": 1}, None)
        .await
        .is_err()
    {
        log_error_and_send_to_discord(
            &conf,
            "[indexer][error]",
            &anyhow::anyhow!("Unable to connect to indexer database"),
        )
        .await;
        return;
    } else {
        log_msg_and_send_to_discord(&conf, "[indexer]", "connected to database").await;
    }

    let server = warp::serve(endpoints::is_ready::status_route(shared_order_key.clone()))
        .run((Ipv4Addr::new(0, 0, 0, 0), conf.indexer_server.port));

    tokio::select! {
        _ = async {
            let mut cursor_opt = None;
            loop {
                let apibara_conf = apibara::create_apibara_config(&conf);
                let uri: Uri = conf.apibara.stream.parse().unwrap();
                let (mut data_stream, data_client) = ClientBuilder::<Filter, Block>::default()
                    .with_bearer_token(conf.apibara.token.clone())
                    .connect(uri)
                    .await
                    .unwrap();

                data_client.send(apibara_conf).await.unwrap();
                println!("[indexer] started");
                match processing::process_data_stream(
                    &mut data_stream,
                    &conf,
                    &shared_state,
                    &shared_order_key,
                )
                .await
                {
                    Err(e) => {
                        log_error_and_send_to_discord(
                            &conf,
                            "[indexer][error]",
                            &anyhow::anyhow!(format!("Error while processing data stream: {:?}", e)),
                        )
                        .await;
                        if let Some(ProcessingError::CursorError(cursor_opt2)) =
                            e.downcast_ref::<ProcessingError>()
                        {
                            cursor_opt = cursor_opt2.clone();
                        }
                    }
                    Ok(_) => {
                        break;
                    }
                }
            }
        } => {}
        _ = server => {}
    }
}
