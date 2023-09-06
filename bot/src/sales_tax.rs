use crate::{config::Config, discord::log_error_and_send_to_discord, models::States};
use std::{collections::HashMap, fs};

pub async fn load_sales_tax(conf: &Config) -> States {
    match fs::read_to_string("./bot/src/sales_tax.json") {
        Ok(data) => match serde_json::from_str(&data) {
            Ok(states) => states,
            Err(e) => {
                log_error_and_send_to_discord(
                    &conf,
                    "[bot][error]",
                    &anyhow::anyhow!("Unable to parse sales tax file: {:?}", e),
                )
                .await;
                States {
                    states: HashMap::new(),
                }
            }
        },
        Err(e) => {
            log_error_and_send_to_discord(
                &conf,
                "[bot][error]",
                &anyhow::anyhow!("Unable to load sales tax file {:?}", e),
            )
            .await;
            States {
                states: HashMap::new(),
            }
        }
    }
}
