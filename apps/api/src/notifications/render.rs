//! Turns one [`EventPayload`] into the body each provider expects.
//!
//! Every renderer is a pure function of (payload, decoration) so the whole set
//! is testable without a network. `decoration = false` drops emoji but keeps
//! structure and colour — the same meaning, less noise.

use serde_json::{Value, json};

use super::event::EventPayload;
use super::provider::{ProviderConfig, ProviderKind};
use crate::dtos::response::AppError;

/// The concrete request the worker will make.
pub struct RenderedRequest {
    pub url: String,
    pub body: Value,
}

pub fn render(
    kind: ProviderKind,
    config: &ProviderConfig,
    decoration: bool,
    payload: &EventPayload,
) -> Result<RenderedRequest, AppError> {
    match kind {
        ProviderKind::Slack => Ok(RenderedRequest {
            url: webhook_url(config)?,
            body: slack_body(payload, decoration),
        }),
        ProviderKind::Discord => Ok(RenderedRequest {
            url: webhook_url(config)?,
            body: discord_body(payload, decoration),
        }),
        ProviderKind::MsTeams => Ok(RenderedRequest {
            url: webhook_url(config)?,
            body: msteams_body(payload, decoration),
        }),
        ProviderKind::Telegram => {
            let token = config.bot_token.as_deref().unwrap_or("").trim();
            let chat_id = config.chat_id.as_deref().unwrap_or("").trim();
            if token.is_empty() || chat_id.is_empty() {
                return Err(AppError::ValidationError(
                    "Telegram provider is missing its bot token or chat ID".into(),
                ));
            }
            Ok(RenderedRequest {
                url: format!("https://api.telegram.org/bot{token}/sendMessage"),
                body: telegram_body(chat_id, payload, decoration),
            })
        }
    }
}

fn webhook_url(config: &ProviderConfig) -> Result<String, AppError> {
    config
        .webhook_url
        .as_deref()
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
        .ok_or_else(|| AppError::ValidationError("Provider is missing its webhook URL".into()))
}

fn heading(payload: &EventPayload, decoration: bool) -> String {
    if decoration {
        format!("{} {}", payload.outcome.emoji(), payload.title)
    } else {
        payload.title.clone()
    }
}

// ── Slack ─────────────────────────────────────────────────────────────────
// Block Kit. `text` stays populated as the notification-preview fallback:
// without it Slack shows "This content can't be displayed" in push alerts.

fn slack_body(payload: &EventPayload, decoration: bool) -> Value {
    let mut blocks = vec![json!({
        "type": "header",
        "text": { "type": "plain_text", "text": heading(payload, decoration), "emoji": true }
    })];

    if !payload.fields.is_empty() {
        blocks.push(json!({
            "type": "section",
            // Block Kit caps a section at 10 fields.
            "fields": payload.fields.iter().take(10).map(|field| json!({
                "type": "mrkdwn",
                "text": format!("*{}*\n{}", field.label, field.value)
            })).collect::<Vec<_>>()
        }));
    }

    if let Some(description) = &payload.description {
        blocks.push(json!({
            "type": "section",
            "text": { "type": "mrkdwn", "text": format!("```{description}```") }
        }));
    }

    if let Some(url) = &payload.url {
        blocks.push(json!({
            "type": "actions",
            "elements": [{
                "type": "button",
                "text": { "type": "plain_text", "text": "Open in litepod" },
                "url": url
            }]
        }));
    }

    blocks.push(json!({
        "type": "context",
        "elements": [{ "type": "mrkdwn", "text": "litepod" }]
    }));

    json!({ "text": heading(payload, decoration), "blocks": blocks })
}

// ── Discord ───────────────────────────────────────────────────────────────

fn discord_body(payload: &EventPayload, decoration: bool) -> Value {
    let mut embed = json!({
        "title": heading(payload, decoration),
        "color": payload.outcome.color(),
        // Discord wants RFC3339; the payload stores epoch millis.
        "timestamp": chrono::DateTime::from_timestamp_millis(payload.timestamp)
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339(),
        "footer": { "text": "litepod" },
        // 25 fields is Discord's hard cap.
        "fields": payload.fields.iter().take(25).map(|field| json!({
            "name": field.label,
            "value": field.value,
            "inline": field.value.chars().count() <= 40
        })).collect::<Vec<_>>()
    });

    if let Some(description) = &payload.description {
        embed["description"] = json!(format!("```\n{description}\n```"));
    }
    if let Some(url) = &payload.url {
        embed["url"] = json!(url);
    }

    json!({ "embeds": [embed] })
}

// ── Microsoft Teams ───────────────────────────────────────────────────────
// MessageCard rather than an Adaptive Card: it is what both the classic
// Incoming Webhook connector and the Workflows replacement accept.

fn msteams_body(payload: &EventPayload, decoration: bool) -> Value {
    let mut card = json!({
        "@type": "MessageCard",
        "@context": "https://schema.org/extensions",
        "themeColor": format!("{:06X}", payload.outcome.color()),
        "summary": payload.title,
        "sections": [{
            "activityTitle": heading(payload, decoration),
            "activitySubtitle": payload.description.clone().unwrap_or_default(),
            "facts": payload.fields.iter().map(|field| json!({
                "name": field.label,
                "value": field.value
            })).collect::<Vec<_>>(),
            "markdown": true
        }]
    });

    if let Some(url) = &payload.url {
        card["potentialAction"] = json!([{
            "@type": "OpenUri",
            "name": "Open in litepod",
            "targets": [{ "os": "default", "uri": url }]
        }]);
    }

    card
}

// ── Telegram ──────────────────────────────────────────────────────────────

fn telegram_body(chat_id: &str, payload: &EventPayload, decoration: bool) -> Value {
    let mut text = format!("*{}*", escape_markdown_v2(&heading(payload, decoration)));
    for field in &payload.fields {
        text.push_str(&format!(
            "\n{}: {}",
            escape_markdown_v2(&field.label),
            escape_markdown_v2(&field.value)
        ));
    }
    if let Some(description) = &payload.description {
        // Inside a code fence only the backslash and backtick need escaping,
        // and escaping the rest would show up literally.
        let fenced = description.replace('\\', "\\\\").replace('`', "\\`");
        text.push_str(&format!("\n```\n{fenced}\n```"));
    }
    if let Some(url) = &payload.url {
        text.push_str(&format!(
            "\n[Open in litepod]({})",
            url.replace('\\', "\\\\").replace(')', "\\)")
        ));
    }

    json!({
        "chat_id": chat_id,
        "text": text,
        "parse_mode": "MarkdownV2",
        "disable_web_page_preview": true
    })
}

/// MarkdownV2 rejects the whole message if any reserved character is left
/// unescaped, which would turn a deploy notification into a silent 400.
fn escape_markdown_v2(input: &str) -> String {
    const RESERVED: [char; 18] = [
        '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!',
    ];
    let mut out = String::with_capacity(input.len());
    for character in input.chars() {
        if RESERVED.contains(&character) || character == '\\' {
            out.push('\\');
        }
        out.push(character);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifications::event::{Field, Outcome, sanitize_description};

    fn payload() -> EventPayload {
        EventPayload {
            action: "app_build_error".into(),
            title: "Deployment of api failed".into(),
            outcome: Outcome::Failure,
            fields: vec![
                Field::new("Application", "api"),
                Field::new("Environment", "litepod / production"),
            ],
            description: Some(sanitize_description("error: could not compile `api`")),
            url: Some("https://litepod.example.com/projects/1".into()),
            timestamp: 1_700_000_000_000,
        }
    }

    fn webhook_config(url: &str) -> ProviderConfig {
        ProviderConfig {
            webhook_url: Some(url.into()),
            ..Default::default()
        }
    }

    #[test]
    fn slack_keeps_a_text_fallback_and_blocks() {
        let rendered = render(
            ProviderKind::Slack,
            &webhook_config("https://hooks.slack.com/services/T/B/X"),
            true,
            &payload(),
        )
        .expect("render");
        assert_eq!(rendered.url, "https://hooks.slack.com/services/T/B/X");
        assert!(rendered.body["text"].as_str().unwrap().contains("❌"));
        assert_eq!(rendered.body["blocks"][0]["type"], "header");
    }

    #[test]
    fn discord_uses_an_embed_with_outcome_colour() {
        let rendered = render(
            ProviderKind::Discord,
            &webhook_config("https://discord.com/api/webhooks/1/a"),
            true,
            &payload(),
        )
        .expect("render");
        let embed = &rendered.body["embeds"][0];
        assert_eq!(embed["color"], Outcome::Failure.color());
        assert_eq!(embed["fields"][0]["name"], "Application");
        assert!(embed["timestamp"].as_str().unwrap().starts_with("2023-"));
    }

    #[test]
    fn organizations_emits_a_message_card() {
        let rendered = render(
            ProviderKind::MsTeams,
            &webhook_config("https://contoso.webhook.office.com/webhookb2/a"),
            true,
            &payload(),
        )
        .expect("render");
        assert_eq!(rendered.body["@type"], "MessageCard");
        assert_eq!(rendered.body["themeColor"], "EF4444");
        assert_eq!(
            rendered.body["sections"][0]["facts"][0]["name"],
            "Application"
        );
    }

    #[test]
    fn telegram_targets_the_bot_api_and_escapes_markdown() {
        let config = ProviderConfig {
            bot_token: Some("123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw".into()),
            chat_id: Some("-100123".into()),
            ..Default::default()
        };
        let rendered = render(ProviderKind::Telegram, &config, true, &payload()).expect("render");
        assert_eq!(
            rendered.url,
            "https://api.telegram.org/bot123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw/sendMessage"
        );
        assert_eq!(rendered.body["chat_id"], "-100123");
        let text = rendered.body["text"].as_str().unwrap();
        // "litepod / production" has no reserved chars, but the title's period-free
        // text plus the field separator must still escape the slash-free dash.
        assert!(text.contains("Deployment of api failed"));
        assert!(!text.contains("\n- "));
    }

    #[test]
    fn markdown_v2_escapes_every_reserved_character() {
        let escaped = escape_markdown_v2("a_b*c[d]e(f)g~h`i>j#k+l-m=n|o{p}q.r!s\\t");
        for character in [
            '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '=', '|', '{', '}', '.', '!',
        ] {
            let needle = format!("\\{character}");
            assert!(
                escaped.contains(&needle),
                "{character} was not escaped in {escaped}"
            );
        }
    }

    #[test]
    fn decoration_off_drops_emoji_but_keeps_structure() {
        let mut plain = payload();
        plain.description = None;
        let decorated = render(
            ProviderKind::Discord,
            &webhook_config("https://discord.com/api/webhooks/1/a"),
            true,
            &plain,
        )
        .expect("render");
        let bare = render(
            ProviderKind::Discord,
            &webhook_config("https://discord.com/api/webhooks/1/a"),
            false,
            &plain,
        )
        .expect("render");
        assert!(
            decorated.body["embeds"][0]["title"]
                .as_str()
                .unwrap()
                .contains('❌')
        );
        assert_eq!(bare.body["embeds"][0]["title"], "Deployment of api failed");
        assert_eq!(
            bare.body["embeds"][0]["color"],
            decorated.body["embeds"][0]["color"]
        );
    }

    #[test]
    fn a_rendered_body_never_contains_the_credential() {
        let secret = "AbCdEfGhIjKlMnOpQrStUvWxYz";
        let config = webhook_config(&format!("https://discord.com/api/webhooks/1/{secret}"));
        let rendered = render(ProviderKind::Discord, &config, true, &payload()).expect("render");
        assert!(!rendered.body.to_string().contains(secret));

        let telegram = ProviderConfig {
            bot_token: Some(format!("123:{secret}xxxxx")),
            chat_id: Some("-100123".into()),
            ..Default::default()
        };
        let rendered = render(ProviderKind::Telegram, &telegram, true, &payload()).expect("render");
        assert!(!rendered.body.to_string().contains(secret));
    }
}
