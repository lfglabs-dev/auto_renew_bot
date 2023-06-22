use anyhow::Result;
use starknet::{
    core::types::{BlockId, BlockStatus},
    providers::Provider,
};

use crate::{bot::get_provider, config::Config};

pub async fn check_block_status(conf: &Config, block_nb: String) -> Result<bool> {
    let provider = get_provider(&conf);
    let block_nb: u64 = match block_nb.parse() {
        Ok(block_nb) => block_nb,
        Err(e) => {
            println!("Unable to parse block number: {}", e);
            return Ok(false);
        }
    };

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

pub async fn get_status_from_endpoint(conf: &Config) -> Result<String> {
    let response = reqwest::get(format!(
        "{}:{}/is_ready",
        conf.indexer_server.server_url, conf.indexer_server.port
    ))
    .await?
    .json()
    .await?;
    Ok(response)
}
