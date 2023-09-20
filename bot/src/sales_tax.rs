use crate::{logger::Logger, models::States};
use std::{collections::HashMap, fs};

pub async fn load_sales_tax(logger: &Logger) -> States {
    match fs::read_to_string("./bot/src/sales_tax.json") {
        Ok(data) => match serde_json::from_str(&data) {
            Ok(states) => states,
            Err(e) => {
                logger.severe(format!("Unable to parse sales tax file: {}", e));
                States {
                    states: HashMap::new(),
                }
            }
        },
        Err(e) => {
            logger.severe(format!("Unable to load sales tax file: {}", e));
            States {
                states: HashMap::new(),
            }
        }
    }
}
