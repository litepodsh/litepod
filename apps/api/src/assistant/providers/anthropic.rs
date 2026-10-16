//! Anthropic Messages compatible rig model construction.

use rig::client::CompletionClient;

use crate::{
    assistant::{config::AssistantConfig, providers::CompletionModelBox},
    dtos::response::AppError,
};

#[allow(dead_code)] // dispatched from providers::build once the rig turn is active.
pub fn build(config: &AssistantConfig, model: &str) -> Result<CompletionModelBox, AppError> {
    let client = rig::providers::anthropic::Client::builder()
        .api_key(config.api_key.clone().unwrap_or_default())
        .base_url(&config.base_url)
        .http_client(super::http_client()?)
        .build()
        .map_err(|error| AppError::ValidationError(error.to_string()))?;
    Ok(rig::agent::ModelHandle::new(client.completion_model(model)))
}
