use std::sync::{Arc, Mutex};

use crate::apibara::{
    ADDRESS_TO_DOMAIN_UPDATE, APPROVAL, DOMAIN_TO_ADDRESS_UPDATE, DOMAIN_TRANSFER,
    STARKNET_ID_UPDATE, TOGGLED_RENEWAL,
};
use crate::apibara_utils::invalidate;
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
    shared_order_key: &Arc<Mutex<Option<String>>>,
) -> Result<()> {
    let mut cursor_opt = None;
    let mut pending_received = false;
    loop {
        let Ok(expected_data) = data_stream.try_next().await else {
            return Err(anyhow::anyhow!(ProcessingError::CursorError(cursor_opt)));
        };
        let Some(message) = expected_data else {
            continue;
        };
        match message {
            DataMessage::Data {
                cursor,
                end_cursor,
                finality,
                batch,
            } => {
                {
                    let mut order_key = shared_order_key.lock().unwrap();
                    *order_key = Some(end_cursor.order_key.clone().to_string());
                }
                let order_key = cursor.clone().unwrap().order_key;

                if conf.devnet_provider.is_devnet {
                    for block in batch.clone() {
                        process_block(&conf, &state, block, order_key).await?;
                        cursor_opt = Some(end_cursor.clone());
                    }
                } else {
                    if finality == DataFinality::DataStatusPending {
                        invalidate(state, cursor.clone()).await;
                        pending_received = true;
                    }
                    if finality == DataFinality::DataStatusAccepted && pending_received {
                        invalidate(state, cursor).await;
                        pending_received = false;
                    }
                    for block in batch {
                        process_block(&conf, &state, block, order_key).await?;
                        cursor_opt = Some(end_cursor.clone());
                    }
                }
            }
            DataMessage::Invalidate { cursor } => {
                invalidate(&state, cursor.clone()).await;
                log_error_and_send_to_discord(
                    &conf,
                    "[indexer][error]",
                    &anyhow::anyhow!(
                        "Chain reorganization detected: {cursor:?}, called invalidate(cursor)"
                    ),
                )
                .await;
            }
            DataMessage::Heartbeat => {}
        }
    }
}

async fn process_block(
    conf: &config::Config,
    state: &Arc<AppState>,
    block: Block,
    order_key: u64,
) -> Result<()> {
    let header = block.header.unwrap_or_default();
    let timestamp: DateTime<Utc> = header.timestamp.unwrap_or_default().try_into()?;

    for event_with_tx in block.events {
        let event = event_with_tx.event.unwrap_or_default();
        let key = &event.keys[0];

        if key == &*ADDRESS_TO_DOMAIN_UPDATE {
            if let Err(e) =
                listeners::addr_to_domain_update(conf, state, &event.data, order_key).await
            {
                println!("Error in addr_to_domain_update: {:?}", e);
            };
        } else if key == &*DOMAIN_TO_ADDRESS_UPDATE {
            if let Err(e) =
                listeners::domain_to_addr_update(conf, state, &event.data, order_key).await
            {
                println!("Error in domain_to_addr_update: {:?}", e);
            }
        } else if key == &*STARKNET_ID_UPDATE {
            if let Err(e) =
                listeners::on_starknet_id_update(conf, state, &event.data, timestamp, order_key)
                    .await
            {
                println!("Error in on_starknet_id_update: {:?}", e);
            }
        } else if key == &*DOMAIN_TRANSFER {
            if let Err(e) = listeners::domain_transfer(conf, state, &event.data, order_key).await {
                println!("Error in domain_transfer: {:?}", e);
            }
        } else if key == &*TOGGLED_RENEWAL {
            if let Err(e) = listeners::toggled_renewal(conf, state, &event.data, order_key).await {
                println!("Error in toggled_renewal: {:?}", e);
            }
        } else if key == &*APPROVAL {
            if let Err(e) = listeners::approval_update(conf, state, &event.data, order_key).await {
                println!("Error in approval_update: {:?}", e);
            }
        }
    }

    Ok(())
}
