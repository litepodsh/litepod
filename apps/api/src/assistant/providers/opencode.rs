//! OpenCode's two endpoint dialects with its conversation cache header.

use crate::{
    assistant::{
        config::{AssistantConfig, EndpointFamily, endpoint_family_for},
        providers::{CompletionModelBox, session_transport::SessionTransport},
    },
    dtos::response::AppError,
};

pub fn build(
    config: &AssistantConfig,
    session: &str,
    model: &str,
) -> Result<CompletionModelBox, AppError> {
    let transport = SessionTransport::new(super::http_client()?, session);
    let key = config.api_key.clone().unwrap_or_default();
    match endpoint_family_for(true, model) {
        EndpointFamily::ChatCompletions => {
            let client = rig::providers::openai::CompletionsClient::builder()
                .api_key(key)
                .base_url(&config.base_url)
                .http_client(transport)
                .build()
                .map_err(|error| AppError::ValidationError(error.to_string()))?;
            Ok(rig::agent::ModelHandle::new(
                rig::providers::openai::CompletionModel::new(client, model),
            ))
        }
        EndpointFamily::Messages => {
            let client = rig::providers::anthropic::Client::builder()
                .api_key(key)
                .base_url(&config.base_url)
                .http_client(transport)
                .build()
                .map_err(|error| AppError::ValidationError(error.to_string()))?;
            Ok(rig::agent::ModelHandle::new(
                rig::providers::anthropic::completion::CompletionModel::with_model(client, model),
            ))
        }
        EndpointFamily::Responses | EndpointFamily::Unsupported => Err(AppError::ValidationError(
            format!("`{model}` uses an endpoint unsupported by this build"),
        )),
    }
}
