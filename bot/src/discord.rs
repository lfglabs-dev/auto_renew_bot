use anyhow::{Context, Result};
use serde_json::json;
use serenity::http::Http;
use serenity::model::channel::Message;
use starknet::core::types::FieldElement;

use crate::config::Config;

pub async fn send_message_to_discord(config: &Config, message: &str) -> Result<Message> {
    let http = Http::new(&config.discord.token);

    let message_json = json!({
        "content": message,
    });
    let message = http
        .send_message(config.discord.channel_id, &message_json)
        .await
        .context("Failed to send message to Discord channel")?;

    Ok(message)
}

pub async fn log_error_and_send_to_discord(
    config: &Config,
    error_type: &str,
    error: &anyhow::Error,
) {
    let message = format!("***{}***: {}", error_type, error);
    println!("{}", message);

    if let Err(e) = send_message_to_discord(&config, &message).await {
        println!("Failed to send error message to Discord: {:?}", e);
    }
}

pub async fn log_msg_and_send_to_discord(config: &Config, log_type: &str, msg_content: &str) {
    let message = format!("_{}_: {}", log_type, msg_content);
    println!("{}", message);

    if let Err(e) = send_message_to_discord(&config, &message).await {
        println!("Failed to send log message to Discord: {:?}", e);
    }
}

pub async fn log_domains_renewed(
    config: &Config,
    domains: (Vec<FieldElement>, Vec<FieldElement>),
) -> Result<()> {
    let message = format!(
        "Domains renewed: \n {}",
        domains
            .0
            .iter()
            .zip(domains.1.iter())
            .map(|(d, r)| format!("- `{}` by `{}`", &starknet::id::decode(*d), r))
            .collect::<Vec<String>>()
            .join(" \n")
    );
    log_msg_and_send_to_discord(&config, "[Renewal]", &message).await;
    Ok(())
}
