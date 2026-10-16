//! Fan-out and the single-shot HTTP send.
//!
//! `enqueue` is what emitting code calls: it resolves the subscribed providers
//! and writes one `pending` row per provider, then returns. It never performs
//! network I/O, so a deploy is never slowed down — or failed — by a webhook.
//! `send_once` is the network half, shared by the worker and the "Test
//! Notification" button.

use std::time::Duration;

use crate::dtos::response::AppError;
use crate::state::state::SharedState;

use super::event::{Action, Event, EventPayload, Scope};
use super::provider::{ProviderConfig, ProviderKind};
use super::render;
use super::repository;
use super::url_guard;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_ERROR_LEN: usize = 500;

/// Outcome of one HTTP attempt.
pub enum SendOutcome {
    Delivered,
    /// Provider asked us to slow down; not counted as a failed attempt.
    RateLimited {
        retry_after: Duration,
    },
    /// Worth retrying: timeouts, connection resets, 5xx.
    Transient(String),
    /// Will never succeed as configured: 4xx, a blocked host, a bad body.
    Permanent(String),
}

/// A dedicated client: redirects are disabled because following one would let
/// an allowlisted host bounce us to an arbitrary internal address, undoing
/// `url_guard`.
fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("litepod-notifications")
        .build()
        .map_err(|error| format!("http client init: {error}"))
}

fn clamp(message: String) -> String {
    if message.chars().count() <= MAX_ERROR_LEN {
        return message;
    }
    let mut out: String = message.chars().take(MAX_ERROR_LEN).collect();
    out.push('…');
    out
}

/// Renders and posts one notification. Runs `url_guard` immediately before the
/// request so a DNS record that changed since the provider was saved cannot be
/// used to reach an internal service.
pub async fn send_once(
    kind: ProviderKind,
    config: &ProviderConfig,
    decoration: bool,
    payload: &EventPayload,
) -> SendOutcome {
    let rendered = match render::render(kind, config, decoration, payload) {
        Ok(rendered) => rendered,
        Err(error) => {
            tracing::warn!(
                provider = kind.key(),
                action = %payload.action,
                error = %error,
                "notification could not be rendered"
            );
            return SendOutcome::Permanent(error.to_string());
        }
    };

    let url = match url::Url::parse(&rendered.url) {
        Ok(url) => url,
        Err(error) => return SendOutcome::Permanent(format!("invalid endpoint: {error}")),
    };
    // Everything logged from here on identifies the destination by host and, for
    // Telegram, chat id. The bot token and the webhook path are the credential —
    // they never reach a log line.
    let host = url.host_str().unwrap_or("unknown").to_string();
    let target = describe_target(kind, config, &host);

    if kind.uses_webhook_url() {
        if let Err(error) = url_guard::validate_webhook_url(kind, &rendered.url) {
            tracing::warn!(
                provider = kind.key(),
                target = %target,
                error = %error,
                "notification blocked by the host allowlist"
            );
            return SendOutcome::Permanent(error);
        }
    }
    if let Err(error) = url_guard::assert_public_host(&url).await {
        tracing::warn!(
            provider = kind.key(),
            target = %target,
            error = %error,
            "notification blocked: destination is not a public address"
        );
        return SendOutcome::Permanent(error);
    }

    let client = match client() {
        Ok(client) => client,
        Err(error) => return SendOutcome::Transient(error),
    };

    tracing::info!(
        provider = kind.key(),
        target = %target,
        action = %payload.action,
        "sending notification"
    );
    let started = std::time::Instant::now();

    let response = match client.post(url).json(&rendered.body).send().await {
        Ok(response) => response,
        Err(error) if error.is_timeout() => {
            tracing::warn!(
                provider = kind.key(),
                target = %target,
                timeout_ms = REQUEST_TIMEOUT.as_millis(),
                "notification request timed out"
            );
            return SendOutcome::Transient(format!("request timed out after {REQUEST_TIMEOUT:?}"));
        }
        Err(error) => {
            tracing::warn!(
                provider = kind.key(),
                target = %target,
                error = %error,
                "notification request failed before a response"
            );
            return SendOutcome::Transient(clamp(error.to_string()));
        }
    };

    let status = response.status();
    let elapsed_ms = started.elapsed().as_millis();
    if status.is_success() {
        tracing::info!(
            provider = kind.key(),
            target = %target,
            action = %payload.action,
            status = status.as_u16(),
            elapsed_ms,
            "notification delivered"
        );
        return SendOutcome::Delivered;
    }

    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    let body = response.text().await.unwrap_or_default();
    // The provider's raw body is logged verbatim: it is the only thing that says
    // *why*, and Telegram/Discord never put a credential in an error response.
    tracing::warn!(
        provider = kind.key(),
        target = %target,
        action = %payload.action,
        status = status.as_u16(),
        elapsed_ms,
        response = %body.trim(),
        "notification rejected by provider"
    );
    classify_for(kind, status.as_u16(), &body, retry_after)
}

/// Destination description safe to log. The webhook path segment is the
/// credential for Slack/Discord/Teams, so only the host is kept; Telegram's chat
/// id is not a secret and is the field most likely to be wrong.
fn describe_target(kind: ProviderKind, config: &ProviderConfig, host: &str) -> String {
    match kind {
        ProviderKind::Telegram => format!(
            "{host} chat_id={}",
            config.chat_id.as_deref().unwrap_or("<unset>")
        ),
        _ => host.to_string(),
    }
}

/// Maps an HTTP response onto the retry policy. Split out from `send_once` so
/// the decision — which failures are worth retrying and which are permanent —
/// is testable without a network.
pub fn classify(status: u16, body: &str, retry_after: Option<u64>) -> SendOutcome {
    if (200..300).contains(&status) {
        return SendOutcome::Delivered;
    }
    if status == 429 {
        return SendOutcome::RateLimited {
            retry_after: Duration::from_secs(retry_after.unwrap_or(30).clamp(1, 300)),
        };
    }

    // The provider's own error text is far more useful than "request failed" —
    // Discord says "Invalid Webhook Token", Slack says "no_service". Surface it.
    let message = clamp(if body.trim().is_empty() {
        format!("HTTP {status}")
    } else {
        format!("HTTP {status} — {}", body.trim())
    });

    // 5xx is the provider having a bad minute; 4xx is our configuration being
    // wrong, and retrying it 5 times just delays the error the user needs.
    if (500..600).contains(&status) {
        SendOutcome::Transient(message)
    } else {
        SendOutcome::Permanent(message)
    }
}

/// `classify` plus a plain-language hint for the rejections users actually hit.
/// The provider's wording is accurate but rarely names the field to fix —
/// Telegram's "the bot can't send messages to the bot" means the chat id is the
/// bot's own id, which is not obvious from the text.
pub fn classify_for(
    kind: ProviderKind,
    status: u16,
    body: &str,
    retry_after: Option<u64>,
) -> SendOutcome {
    let outcome = classify(status, body, retry_after);
    let Some(hint) = hint_for(kind, status, body) else {
        return outcome;
    };
    match outcome {
        SendOutcome::Permanent(message) => {
            SendOutcome::Permanent(clamp(format!("{message} — {hint}")))
        }
        SendOutcome::Transient(message) => {
            SendOutcome::Transient(clamp(format!("{message} — {hint}")))
        }
        other => other,
    }
}

fn hint_for(kind: ProviderKind, status: u16, body: &str) -> Option<&'static str> {
    let body = body.to_ascii_lowercase();
    match kind {
        ProviderKind::Telegram => {
            if body.contains("can't send messages to the bot") {
                Some(
                    "the Chat ID is the bot's own id. Use the id of the chat, group or channel the bot posts to: message the bot, then read chat.id from https://api.telegram.org/bot<token>/getUpdates",
                )
            } else if body.contains("chat not found") {
                Some(
                    "no chat with that id. Add the bot to the chat first, and for groups keep the leading minus sign (e.g. -1001234567890)",
                )
            } else if body.contains("bot was kicked") || body.contains("bot is not a member") {
                Some("the bot is no longer in that chat. Re-add it and grant it permission to post")
            } else if body.contains("not enough rights") {
                Some(
                    "the bot is in the chat but cannot post. Give it the 'Send Messages' permission",
                )
            } else if status == 401 || body.contains("unauthorized") {
                Some("the bot token is wrong or was revoked. Get a fresh one from @BotFather")
            } else if body.contains("can't parse entities") {
                Some(
                    "Telegram rejected the message formatting — please report this, it is a litepod bug",
                )
            } else {
                None
            }
        }
        ProviderKind::Discord => match status {
            401 | 403 => Some(
                "the webhook was deleted or its token changed. Re-copy the URL from Server Settings → Integrations → Webhooks",
            ),
            404 => Some(
                "that webhook no longer exists. Create a new one in the channel and paste its URL",
            ),
            _ => None,
        },
        ProviderKind::Slack => {
            if body.contains("no_service") || body.contains("no_organization") {
                Some("the Slack app or webhook was removed. Re-install it and copy the new URL")
            } else if body.contains("channel_not_found") {
                Some("the channel the webhook points at was archived or deleted")
            } else if status == 403 {
                Some(
                    "Slack rejected this webhook. Re-copy it from the app's Incoming Webhooks page",
                )
            } else {
                None
            }
        }
        ProviderKind::MsTeams => match status {
            400 => Some(
                "Teams rejected the card. If this is a Workflows URL, make sure the flow accepts a 'When a Teams webhook request is received' trigger",
            ),
            401 | 403 | 404 => Some(
                "the connector was removed from the channel. Create it again and paste the new URL",
            ),
            _ => None,
        },
    }
}

/// Resolves subscribers for an event and queues one delivery each.
/// Returns how many rows were written.
pub async fn enqueue(state: &SharedState, event: &Event) -> Result<usize, AppError> {
    let providers = match &event.scope {
        Scope::Organization(organization_id) => {
            repository::subscribers_for_organization(state, organization_id, event.action).await?
        }
        Scope::Global => repository::subscribers_for_global(state, event.action).await?,
    };
    if providers.is_empty() {
        // Worth an explicit line: "I configured a webhook and nothing arrives"
        // is almost always a provider that is disabled or not subscribed.
        tracing::debug!(
            action = event.action.key(),
            scope = match &event.scope {
                Scope::Organization(organization_id) => organization_id.as_str(),
                Scope::Global => "global(root organization)",
            },
            "no enabled notification provider is subscribed to this action"
        );
        return Ok(0);
    }

    let payload_json = serde_json::to_string(&event.payload)
        .map_err(|error| AppError::InternalError(format!("payload encode: {error}")))?;

    let mut queued = 0;
    for provider in providers {
        match repository::enqueue_delivery(state, &provider.id, event.action, &payload_json).await {
            Ok(delivery_id) => {
                queued += 1;
                tracing::info!(
                    action = event.action.key(),
                    provider = provider.kind.key(),
                    provider_name = %provider.name,
                    delivery_id = %delivery_id,
                    "notification queued"
                );
            }
            Err(error) => tracing::warn!(
                provider_id = %provider.id,
                provider_name = %provider.name,
                error = %error,
                "could not queue notification delivery"
            ),
        }
    }
    Ok(queued)
}

/// After this many failed attempts a delivery is dead-lettered.
pub const MAX_ATTEMPTS: i64 = 5;

/// Retry schedule. `attempt` is the attempt that just failed (1-based);
/// `None` means give up.
pub fn backoff(attempt: i64) -> Option<Duration> {
    if attempt >= MAX_ATTEMPTS {
        return None;
    }
    match attempt {
        1 => Some(Duration::from_secs(30)),
        2 => Some(Duration::from_secs(2 * 60)),
        3 => Some(Duration::from_secs(10 * 60)),
        4 => Some(Duration::from_secs(30 * 60)),
        _ => None,
    }
}

/// Provider-agnostic sanity check used by the handler before it persists
/// anything, so a bad config is rejected with a field error rather than
/// discovered by the worker 30 seconds later.
pub fn validate_config(kind: ProviderKind, config: &ProviderConfig) -> Result<(), AppError> {
    config
        .validate(kind)
        .map_err(AppError::ValidationError)
        .map(|_| ())
}

/// Convenience for the handler: is this action one only the root organization may
/// subscribe to?
pub fn requires_root_organization(actions: &[Action]) -> bool {
    actions.iter().any(|action| action.is_global())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_then_gives_up() {
        assert_eq!(backoff(1), Some(Duration::from_secs(30)));
        assert_eq!(backoff(2), Some(Duration::from_secs(120)));
        assert_eq!(backoff(3), Some(Duration::from_secs(600)));
        assert_eq!(backoff(4), Some(Duration::from_secs(1800)));
        assert_eq!(backoff(MAX_ATTEMPTS), None);
        assert_eq!(backoff(99), None);
    }

    #[test]
    fn error_text_is_clamped() {
        let long = "x".repeat(2_000);
        let clamped = clamp(long);
        assert_eq!(clamped.chars().count(), MAX_ERROR_LEN + 1);
    }

    #[test]
    fn response_classification_drives_the_retry_policy() {
        assert!(matches!(classify(204, "", None), SendOutcome::Delivered));
        assert!(matches!(
            classify(429, "", Some(90)),
            SendOutcome::RateLimited { retry_after } if retry_after == Duration::from_secs(90)
        ));
        // An absurd Retry-After must not park a delivery for hours.
        assert!(matches!(
            classify(429, "", Some(86_400)),
            SendOutcome::RateLimited { retry_after } if retry_after == Duration::from_secs(300)
        ));
        assert!(matches!(
            classify(503, "upstream down", None),
            SendOutcome::Transient(_)
        ));
        match classify(401, "{\"message\": \"Invalid Webhook Token\"}", None) {
            SendOutcome::Permanent(message) => {
                assert!(message.contains("401"));
                // The provider's own wording has to survive: it is the only
                // thing that tells the user what to fix.
                assert!(message.contains("Invalid Webhook Token"));
            }
            _ => panic!("a 401 must be permanent"),
        }
    }

    #[test]
    fn provider_rejections_get_an_actionable_hint() {
        // Telegram's own wording for "you used the bot's id as the chat id"
        // names neither the field nor the fix.
        let body = r#"{"ok":false,"error_code":403,"description":"Forbidden: the bot can't send messages to the bot"}"#;
        match classify_for(ProviderKind::Telegram, 403, body, None) {
            SendOutcome::Permanent(message) => {
                assert!(message.contains("the bot can't send messages to the bot"));
                assert!(message.contains("Chat ID is the bot's own id"));
                assert!(message.contains("getUpdates"));
            }
            _ => panic!("403 must be permanent"),
        }

        match classify_for(
            ProviderKind::Discord,
            401,
            "{\"message\":\"Invalid Webhook Token\"}",
            None,
        ) {
            SendOutcome::Permanent(message) => assert!(message.contains("Re-copy the URL")),
            _ => panic!("401 must be permanent"),
        }

        // Unrecognised bodies pass through untouched rather than getting a
        // guess that might send the user down the wrong path.
        match classify_for(ProviderKind::Slack, 400, "something new", None) {
            SendOutcome::Permanent(message) => assert_eq!(message, "HTTP 400 — something new"),
            _ => panic!("400 must be permanent"),
        }
    }

    #[test]
    fn only_global_actions_demand_the_root_organization() {
        assert!(!requires_root_organization(&[
            Action::AppDeploy,
            Action::AppBuildError
        ]));
        assert!(requires_root_organization(&[
            Action::AppDeploy,
            Action::LitepodRestart
        ]));
    }
}
