//! Cloudflare Turnstile server-side verification (siteverify).
//!
//! Only wired to the cloud registration path (`register_handler` when `state.is_cloud`).
//! The widget itself is managed in the Cloudflare dashboard; this module never talks to
//! the Cloudflare account API, only to the public `siteverify` endpoint.

use std::net::IpAddr;

use crate::dtos::response::AppError;

/// Cloud asks for a captcha in release builds only: a debug build (`just api`)
/// never does, so local development needs no Cloudflare keys.
pub fn required(is_cloud: bool) -> bool {
    is_cloud && !cfg!(debug_assertions)
}

const SITEVERIFY_URL: &str = "https://challenges.cloudflare.com/turnstile/v0/siteverify";

/// Verifies a Turnstile response token against `TURNSTILE_SECRET`. Fails closed: a missing
/// secret, network error, non-2xx response, non-JSON body, or `success != true` all reject.
pub async fn verify(token: &str, remote_ip: IpAddr) -> Result<(), AppError> {
    let secret = std::env::var("TURNSTILE_SECRET")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| AppError::InternalError("TURNSTILE_SECRET not configured".to_string()))?;

    let resp = reqwest::Client::new()
        .post(SITEVERIFY_URL)
        .form(&[
            ("secret", secret.as_str()),
            ("response", token),
            ("remoteip", &remote_ip.to_string()),
        ])
        .send()
        .await
        .map_err(|error| {
            tracing::warn!(%error, "turnstile: siteverify request failed (network error)");
            AppError::TurnstileFailed
        })?;

    if !resp.status().is_success() {
        tracing::warn!(status = %resp.status(), "turnstile: siteverify returned non-2xx");
        return Err(AppError::TurnstileFailed);
    }

    let body: serde_json::Value = resp.json().await.map_err(|error| {
        tracing::warn!(%error, "turnstile: siteverify response was not valid JSON");
        AppError::TurnstileFailed
    })?;
    if body.get("success").and_then(|v| v.as_bool()) != Some(true) {
        let error_codes = body.get("error-codes").cloned().unwrap_or_default();
        tracing::warn!(%error_codes, "turnstile: siteverify rejected token");
        return Err(AppError::TurnstileFailed);
    }

    Ok(())
}
