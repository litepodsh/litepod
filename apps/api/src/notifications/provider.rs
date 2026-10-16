//! Provider kinds, their credentials, and how those credentials are stored.
//!
//! Credentials are encrypted with `utils::security::encrypt_data` — the same
//! treatment `git_providers.password_encrypted` gets. The client never sees
//! them back; it sees `config_hint`, a redacted display string, mirroring
//! `S3ConfigDto::has_secret_key`.

use serde::{Deserialize, Serialize};

use crate::dtos::response::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Slack,
    Discord,
    Telegram,
    MsTeams,
}

impl ProviderKind {
    pub const ALL: [ProviderKind; 4] = [
        ProviderKind::Slack,
        ProviderKind::Discord,
        ProviderKind::Telegram,
        ProviderKind::MsTeams,
    ];

    pub fn key(self) -> &'static str {
        match self {
            ProviderKind::Slack => "slack",
            ProviderKind::Discord => "discord",
            ProviderKind::Telegram => "telegram",
            ProviderKind::MsTeams => "msteams",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::Slack => "Slack",
            ProviderKind::Discord => "Discord",
            ProviderKind::Telegram => "Telegram",
            ProviderKind::MsTeams => "Microsoft Teams",
        }
    }

    pub fn parse(value: &str) -> Option<ProviderKind> {
        ProviderKind::ALL.into_iter().find(|k| k.key() == value)
    }

    /// Telegram is the odd one out: it authenticates with a bot token and
    /// targets a chat id, and the endpoint is derived rather than supplied.
    pub fn uses_webhook_url(self) -> bool {
        !matches!(self, ProviderKind::Telegram)
    }
}

/// What lives inside `notification_providers.config_encrypted`. Optional
/// fields rather than an enum so the stored JSON stays readable and adding a
/// provider with new fields does not invalidate existing rows.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
}

impl ProviderConfig {
    /// Full validation for a config the user just typed. Webhook URLs go
    /// through the allowlist; Telegram credentials get shape checks.
    pub fn validate(&self, kind: ProviderKind) -> Result<(), String> {
        match kind {
            ProviderKind::Telegram => {
                let token = self.bot_token.as_deref().unwrap_or("").trim();
                if token.is_empty() {
                    return Err("Bot token is required".into());
                }
                // BotFather tokens are `<digits>:<base64-ish>`. Checking the
                // shape here turns a silent 404 from Telegram into a clear
                // field error.
                if !is_plausible_bot_token(token) {
                    return Err(
                        "Bot token looks wrong — expected the form 123456789:AA... from @BotFather"
                            .into(),
                    );
                }
                let chat_id = self.chat_id.as_deref().unwrap_or("").trim();
                if chat_id.is_empty() {
                    return Err("Chat ID is required".into());
                }
                if !is_plausible_chat_id(chat_id) {
                    return Err(
                        "Chat ID must be a number (e.g. -1001234567890) or a @channelname".into(),
                    );
                }
                Ok(())
            }
            _ => {
                let raw = self.webhook_url.as_deref().unwrap_or("");
                super::url_guard::validate_webhook_url(kind, raw).map(|_| ())
            }
        }
    }

    /// Safe-to-return summary. Shows enough to tell two providers apart and
    /// never enough to post to the channel.
    pub fn hint(&self, kind: ProviderKind) -> String {
        match kind {
            ProviderKind::Telegram => {
                let chat = self.chat_id.as_deref().unwrap_or("");
                format!("chat {chat}")
            }
            _ => {
                let raw = self.webhook_url.as_deref().unwrap_or("");
                redact_webhook_url(raw)
            }
        }
    }

    pub fn encrypt(&self) -> Result<String, AppError> {
        let json = serde_json::to_string(self)
            .map_err(|error| AppError::InternalError(format!("config encode: {error}")))?;
        let key = crate::utils::security::get_encryption_key();
        crate::utils::security::encrypt_data(&json, &key)
            .map_err(|error| AppError::InternalError(format!("config encrypt: {error}")))
    }

    pub fn decrypt(blob: &str) -> Result<ProviderConfig, AppError> {
        let key = crate::utils::security::get_encryption_key();
        let json = crate::utils::security::decrypt_data(blob, &key)
            .map_err(|error| AppError::InternalError(format!("config decrypt: {error}")))?;
        serde_json::from_str(&json)
            .map_err(|error| AppError::InternalError(format!("config decode: {error}")))
    }
}

fn is_plausible_bot_token(token: &str) -> bool {
    let Some((id, secret)) = token.split_once(':') else {
        return false;
    };
    !id.is_empty()
        && id.chars().all(|c| c.is_ascii_digit())
        && secret.len() >= 20
        && secret
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn is_plausible_chat_id(chat_id: &str) -> bool {
    if let Some(name) = chat_id.strip_prefix('@') {
        return name.len() >= 3 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    }
    let digits = chat_id.strip_prefix('-').unwrap_or(chat_id);
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

/// `https://discord.com/api/webhooks/123456/AbCdEf...` becomes
/// `discord.com/api/webhooks/…/AbCd…`. Keeps the host and a four-character
/// tail so the user can recognise which webhook this is.
fn redact_webhook_url(raw: &str) -> String {
    let Ok(url) = url::Url::parse(raw) else {
        return String::new();
    };
    let host = url.host_str().unwrap_or("");
    let segments: Vec<&str> = url.path().split('/').filter(|s| !s.is_empty()).collect();
    let Some((last, leading)) = segments.split_last() else {
        return host.to_string();
    };
    // Everything but the final secret segment is structural (`api/webhooks`,
    // `services`, `webhookb2`) plus at most one id, which is not a credential
    // on its own but is still trimmed to a stub.
    let prefix: Vec<String> = leading
        .iter()
        .map(|segment| {
            if segment.len() > 12 {
                "…".to_string()
            } else {
                (*segment).to_string()
            }
        })
        .collect();
    let tail: String = last
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{host}/{}/…{tail}", prefix.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_keys_round_trip() {
        for kind in ProviderKind::ALL {
            assert_eq!(ProviderKind::parse(kind.key()), Some(kind));
        }
        assert_eq!(ProviderKind::parse("lark"), None);
    }

    #[test]
    fn telegram_credentials_are_shape_checked() {
        let good = ProviderConfig {
            bot_token: Some("123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw".into()),
            chat_id: Some("-1001234567890".into()),
            ..Default::default()
        };
        assert!(good.validate(ProviderKind::Telegram).is_ok());

        let channel = ProviderConfig {
            chat_id: Some("@litepod_alerts".into()),
            ..good.clone()
        };
        assert!(channel.validate(ProviderKind::Telegram).is_ok());

        let bad_token = ProviderConfig {
            bot_token: Some("nope".into()),
            ..good.clone()
        };
        assert!(bad_token.validate(ProviderKind::Telegram).is_err());

        let bad_chat = ProviderConfig {
            chat_id: Some("my channel".into()),
            ..good.clone()
        };
        assert!(bad_chat.validate(ProviderKind::Telegram).is_err());
    }

    #[test]
    fn webhook_kinds_defer_to_the_allowlist() {
        let discord = ProviderConfig {
            webhook_url: Some("https://discord.com/api/webhooks/123/AbCdEfGhIjKlMnOp".into()),
            ..Default::default()
        };
        assert!(discord.validate(ProviderKind::Discord).is_ok());
        assert!(discord.validate(ProviderKind::Slack).is_err());
    }

    #[test]
    fn hint_never_leaks_the_secret_segment() {
        let secret = "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789";
        let config = ProviderConfig {
            webhook_url: Some(format!(
                "https://discord.com/api/webhooks/123456789/{secret}"
            )),
            ..Default::default()
        };
        let hint = config.hint(ProviderKind::Discord);
        assert!(!hint.contains(secret));
        assert!(hint.starts_with("discord.com/"));
        assert!(hint.ends_with("6789"));
    }

    #[test]
    fn config_round_trips_through_encryption() {
        let config = ProviderConfig {
            webhook_url: Some("https://hooks.slack.com/services/T0/B0/XXXX".into()),
            ..Default::default()
        };
        let blob = config.encrypt().expect("encrypt");
        assert!(!blob.contains("hooks.slack.com"));
        let back = ProviderConfig::decrypt(&blob).expect("decrypt");
        assert_eq!(back.webhook_url, config.webhook_url);
    }
}
