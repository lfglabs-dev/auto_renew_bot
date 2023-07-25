use anyhow::Result;
use starknet::{
    core::types::{BlockId, BlockStatus},
    providers::Provider,
};

use crate::{bot::get_provider, config::Config};

pub async fn check_block_status(conf: &Config, block_nb: u64) -> Result<bool> {
    let provider = get_provider(&conf);

    match provider.get_block(BlockId::Number(block_nb)).await {
        Ok(block) => {
            if block.status == BlockStatus::AcceptedOnL2 || block.status == BlockStatus::Pending {
                Ok(true)
            } else {
                println!(
                    "Indexer is still processing old data. Block = {}.",
                    block_nb
                );
                Ok(false)
            }
        }
        Err(e) => {
            println!(
                "Error while fetching block status {} with error: {}",
                block_nb, e
            );
            Ok(false)
        }
    }
}

pub async fn get_status_from_endpoint(conf: &Config) -> Result<u64, Box<dyn std::error::Error>> {
    // Perform the request and get the response as a string
    let raw_response = reqwest::get(format!(
        "{}:{}/is_ready",
        conf.indexer_server.server_url, conf.indexer_server.port
    ))
    .await?
    .text()
    .await?;

    // Log the raw response
    println!("Raw response: {}", &raw_response);

    // Now, try to parse the raw response as JSON
    let response: serde_json::Value = serde_json::from_str(&raw_response)?;

    // Extract the "last_block" field as a usize
    let last_block = response["last_block"].as_u64()
        .ok_or("Failed to parse 'last_block' as usize")?;

    Ok(last_block)
}
