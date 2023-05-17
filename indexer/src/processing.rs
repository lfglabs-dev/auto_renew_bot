use std::sync::Arc;

use crate::apibara::{
    ADDRESS_TO_DOMAIN_UPDATE, APPROVAL, DOMAIN_TO_ADDRESS_UPDATE, DOMAIN_TRANSFER,
    STARKNET_ID_UPDATE, TOGGLED_RENEWAL,
};
use crate::config;
use crate::discord::log_error_and_send_to_discord;
use crate::listeners;
use crate::models::AppState;
use anyhow::Result;
use apibara_core::node::v1alpha2::Cursor;
use apibara_core::{
    node::v1alpha2::DataFinality,
    starknet::v1alpha2::{Block, Filter},
};
use apibara_sdk::{DataMessage, DataStream};
use chrono::{DateTime, Utc};
use thiserror::Error;
use tokio_stream::StreamExt;

#[derive(Error, Debug)]
pub enum ProcessingError {
    #[error("Connection reset")]
    CursorError(Option<Cursor>),
}

pub async fn process_data_stream(
    data_stream: &mut DataStream<Filter, Block>,
    conf: &config::Config,
    state: &Arc<AppState>,
) -> Result<()> {
    let mut cursor_opt = None;
    loop {
        let Ok(expected_data) = data_stream.try_next().await else {
            return Err(anyhow::anyhow!(ProcessingError::CursorError(cursor_opt)));
        };
        let Some(message) = expected_data else {
            continue;
        };
        match message {
            DataMessage::Data {
                cursor: _,
                end_cursor,
                finality,
                batch,
            } => {
                if conf.devnet_provider.is_devnet {
                    println!("processing block");
                    for block in batch.clone() {
                        process_block(&conf, &state, block).await?;
                        cursor_opt = Some(end_cursor.clone());
                    }
                }

                // only store blocks that are finalized
                if !conf.devnet_provider.is_devnet && finality == DataFinality::DataStatusFinalized
                {
                    for block in batch {
                        process_block(&conf, &state, block).await?;
                        cursor_opt = Some(end_cursor.clone());
                    }
                }
            }
            DataMessage::Invalidate { cursor } => {
                log_error_and_send_to_discord(
                    &conf,
                    "[indexer][error]",
                    &anyhow::anyhow!("Chain reorganization detected: {cursor:?}"),
                )
                .await;
                panic!("chain reorganization detected: {cursor:?}");
            }
        }
    }
}

async fn process_block(conf: &config::Config, state: &Arc<AppState>, block: Block) -> Result<()> {
    let header = block.header.unwrap_or_default();
    let timestamp: DateTime<Utc> = header.timestamp.unwrap_or_default().try_into()?;

    for event_with_tx in block.events {
        let event = event_with_tx.event.unwrap_or_default();
        let key = &event.keys[0];

        if key == &*ADDRESS_TO_DOMAIN_UPDATE {
            if let Err(e) = listeners::addr_to_domain_update(conf, state, &event.data).await {
                println!("Error in addr_to_domain_update: {:?}", e);
            };
        } else if key == &*DOMAIN_TO_ADDRESS_UPDATE {
            if let Err(e) = listeners::domain_to_addr_update(conf, state, &event.data).await {
                println!("Error in domain_to_addr_update: {:?}", e);
            }
        } else if key == &*STARKNET_ID_UPDATE {
            if let Err(e) =
                listeners::on_starknet_id_update(conf, state, &event.data, timestamp).await
            {
                println!("Error in on_starknet_id_update: {:?}", e);
            }
        } else if key == &*DOMAIN_TRANSFER {
            if let Err(e) = listeners::domain_transfer(conf, state, &event.data).await {
                println!("Error in domain_transfer: {:?}", e);
            }
        } else if key == &*TOGGLED_RENEWAL {
            if let Err(e) = listeners::toggled_renewal(conf, state, &event.data).await {
                println!("Error in toggled_renewal: {:?}", e);
            }
        } else if key == &*APPROVAL {
            if let Err(e) = listeners::approval_update(conf, state, &event.data).await {
                println!("Error in approval_update: {:?}", e);
            }
        }
    }

    Ok(())
}
