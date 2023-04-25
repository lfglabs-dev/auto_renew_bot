use std::sync::Arc;

use crate::apibara::{
    ADDRESS_TO_DOMAIN_UPDATE, DOMAIN_RENEWED, DOMAIN_TO_ADDRESS_UPDATE, DOMAIN_TRANSFER,
    STARKNET_ID_UPDATE, TOGGLED_RENEWAL,
};
use crate::config;
use crate::listeners;
use crate::models::AppState;
use anyhow::Result;
use apibara_core::{
    node::v1alpha2::DataFinality,
    starknet::v1alpha2::{Block, Filter},
};
use apibara_sdk::{DataMessage, DataStream};
use chrono::{DateTime, Utc};
use tokio::time::Duration;
use tokio_stream::StreamExt;

pub async fn process_data_stream(
    data_stream: &mut DataStream<Filter, Block>,
    conf: &config::Config,
    state: &Arc<AppState>,
) -> Result<()> {
    loop {
        match data_stream.try_next().await {
            Ok(Some(message)) => match message {
                DataMessage::Data {
                    cursor: _,
                    end_cursor: _,
                    finality,
                    batch,
                } => {
                    if finality != DataFinality::DataStatusFinalized {
                        println!("shutting down");
                        break;
                    }

                    for block in batch {
                        process_block(&conf, &state, block).await?;
                    }
                }
                DataMessage::Invalidate { cursor } => {
                    panic!("chain reorganization detected: {cursor:?}");
                }
            },
            Ok(None) => {
                break;
            }
            Err(e) => {
                eprintln!("Error while processing data stream: {:?}", e);
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        }
    }

    Ok(())
}

async fn process_block(conf: &config::Config, state: &Arc<AppState>, block: Block) -> Result<()> {
    let header = block.header.unwrap_or_default();
    let timestamp: DateTime<Utc> = header.timestamp.unwrap_or_default().try_into()?;

    for event_with_tx in block.events {
        let event = event_with_tx.event.unwrap_or_default();
        let key = &event.keys[0];

        if key == &*ADDRESS_TO_DOMAIN_UPDATE {
            listeners::addr_to_domain_update(conf, state, &event.data).await;
        } else if key == &*DOMAIN_TO_ADDRESS_UPDATE {
            listeners::domain_to_addr_update(conf, state, &event.data).await;
        } else if key == &*STARKNET_ID_UPDATE {
            listeners::on_starknet_id_update(conf, state, &event.data, timestamp).await;
        } else if key == &*DOMAIN_TRANSFER {
            listeners::domain_transfer(conf, state, &event.data).await;
        } else if key == &*TOGGLED_RENEWAL {
            listeners::toggled_renewal(conf, state, &event.data).await;
        } else if key == &*DOMAIN_RENEWED {
            listeners::domain_renewed(conf, state, &event.data, timestamp).await;
        }
    }

    Ok(())
}
