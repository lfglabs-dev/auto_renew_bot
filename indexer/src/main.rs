extern crate lazy_static;
use std::sync::Arc;

use apibara_core::starknet::v1alpha2::{Block, Filter};
use apibara_sdk::{ClientBuilder, Uri};
use mongodb::{bson::doc, options::ClientOptions, Client};
mod apibara;
mod config;
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

    let apibara_conf = apibara::create_apibara_config(&conf);
    // let uri: Uri = conf.apibara.stream.parse().unwrap();
    let (mut data_stream, data_client) = ClientBuilder::<Filter, Block>::default()
        .connect(Uri::from_static("http://0.0.0.0:7172"))
        .await
        .unwrap();

    data_client.send(apibara_conf).await.unwrap();
    processing::process_data_stream(&mut data_stream, &conf, &shared_state)
        .await
        .unwrap();
}
