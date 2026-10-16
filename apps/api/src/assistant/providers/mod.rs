//! Per-provider rig client construction.

pub mod anthropic;
pub mod openai;
pub mod opencode;
pub mod session_transport;

use crate::{
    assistant::{
        config::{AssistantConfig, EndpointFamily},
        endpoint,
    },
    dtos::response::AppError,
};

/// Rig's concrete erasure boundary for heterogeneous completion models.
#[allow(dead_code)] // consumed by the contextual rig turn entry point next.
pub type CompletionModelBox = rig::agent::ModelHandle;

/// The HTTP client every provider hands rig. Redirects are off: `build` vets the
/// connection's own host, and a public host answering 302 to an internal address
/// (`http://169.254.169.254/`, the Caddy admin API) would otherwise be followed
/// without that check.
pub fn http_client() -> Result<rig::http_client::ReqwestClient, AppError> {
    rig::http_client::ReqwestClient::builder()
        .redirect(reqwest013::redirect::Policy::none())
        .build()
        .map_err(|error| AppError::InternalError(format!("could not build HTTP client: {error}")))
}

/// Construct the rig completion model for one connection.
///
/// `session` is forwarded by every provider even though only the OpenCode
/// dialects consume it today — keeping the call site uniform means adding a
/// new session-aware provider is one entry in the dispatch, not a new
/// signature everywhere `build` is called.
#[allow(dead_code)] // consumed by the contextual rig turn entry point next.
pub async fn build(
    config: &AssistantConfig,
    _session: &str,
    model: &str,
) -> Result<CompletionModelBox, AppError> {
    endpoint::allowed(&config.base_url, config.allow_local)
        .await
        .map_err(AppError::ValidationError)?;

    if config.opencode {
        return opencode::build(config, _session, model);
    }
    let family = {
        match config.protocol {
            crate::assistant::config::Protocol::OpenAi => EndpointFamily::ChatCompletions,
            crate::assistant::config::Protocol::Anthropic => EndpointFamily::Messages,
        }
    };
    match family {
        EndpointFamily::ChatCompletions => openai::build(config, model),
        EndpointFamily::Messages => anthropic::build(config, model),
        EndpointFamily::Responses | EndpointFamily::Unsupported => {
            Err(AppError::ValidationError(format!(
                "`{model}` uses the unsupported {} endpoint",
                family.as_str()
            )))
        }
    }
}
