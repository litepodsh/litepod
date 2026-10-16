// Lok­i log shipping is temporarily disabled (see main.rs). The module is kept
// so the wiring can be re-enabled without re-deriving the batch shape.
#![allow(dead_code)]

use std::{
    fmt,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Map, Value, json};
use tokio::sync::mpsc;
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{Layer, layer::Context};
use url::Url;

const LOKI_PUSH_PATH: &str = "loki/api/v1/push";
const LOG_CHANNEL_CAPACITY: usize = 1_024;
const MAX_BATCH_SIZE: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LokiConfig {
    push_url: Url,
    service: String,
    environment: String,
}

impl LokiConfig {
    pub(crate) fn from_env() -> Result<Option<Self>, url::ParseError> {
        let url = std::env::var("LOKI_URL").ok();
        let service = std::env::var("LOKI_SERVICE").ok();
        let environment = std::env::var("APP_ENV").ok();
        Self::from_values(url.as_deref(), service.as_deref(), environment.as_deref())
    }

    fn from_values(
        url: Option<&str>,
        service: Option<&str>,
        environment: Option<&str>,
    ) -> Result<Option<Self>, url::ParseError> {
        let Some(url) = url.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(None);
        };

        let mut push_url = Url::parse(url)?;
        if push_url.path().trim_end_matches('/') != format!("/{LOKI_PUSH_PATH}") {
            push_url = push_url.join(LOKI_PUSH_PATH)?;
        }

        Ok(Some(Self {
            push_url,
            service: optional_label(service, "litepod"),
            environment: optional_label(environment, "dev"),
        }))
    }
}

fn optional_label(value: Option<&str>, default: &str) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default)
        .to_owned()
}

pub(crate) struct LokiLayer {
    sender: mpsc::Sender<LokiEntry>,
}

impl LokiLayer {
    pub(crate) fn start(config: LokiConfig) -> Self {
        let (sender, receiver) = mpsc::channel(LOG_CHANNEL_CAPACITY);
        tokio::spawn(ship_logs(config, receiver));
        Self { sender }
    }
}

impl<S> Layer<S> for LokiLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut visitor = JsonVisitor::default();
        event.record(&mut visitor);

        let metadata = event.metadata();
        let line = json!({
            "level": metadata.level().as_str(),
            "target": metadata.target(),
            "fields": visitor.fields,
        })
        .to_string();

        let _ = self.sender.try_send(LokiEntry {
            timestamp: unix_time_nanoseconds(),
            line,
        });
    }
}

async fn ship_logs(config: LokiConfig, mut receiver: mpsc::Receiver<LokiEntry>) {
    let client = reqwest::Client::new();

    while let Some(first_entry) = receiver.recv().await {
        let mut values = Vec::with_capacity(MAX_BATCH_SIZE);
        values.push(first_entry.as_loki_value());

        while values.len() < MAX_BATCH_SIZE {
            match receiver.try_recv() {
                Ok(entry) => values.push(entry.as_loki_value()),
                Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) => {
                    break;
                }
            }
        }

        let payload = json!({
            "streams": [{
                "stream": {
                    "service": config.service,
                    "environment": config.environment,
                },
                "values": values,
            }],
        });

        match client
            .post(config.push_url.clone())
            .json(&payload)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {}
            Ok(response) => eprintln!("[loki] log batch rejected with HTTP {}", response.status()),
            Err(error) => eprintln!("[loki] log batch delivery failed: {error}"),
        }
    }
}

struct LokiEntry {
    timestamp: String,
    line: String,
}

impl LokiEntry {
    fn as_loki_value(self) -> [String; 2] {
        [self.timestamp, self.line]
    }
}

fn unix_time_nanoseconds() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string()
}

#[derive(Default)]
struct JsonVisitor {
    fields: Map<String, Value>,
}

impl Visit for JsonVisitor {
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields
            .insert(field.name().to_owned(), Value::Bool(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields.insert(field.name().to_owned(), value.into());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields.insert(field.name().to_owned(), value.into());
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.fields.insert(field.name().to_owned(), json!(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), Value::String(value.to_owned()));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), Value::String(format!("{value:?}")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_is_disabled_without_a_url() {
        assert!(LokiConfig::from_values(None, None, None).unwrap().is_none());
        assert!(
            LokiConfig::from_values(Some("  \t "), None, None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn configuration_uses_push_endpoint_and_default_labels() {
        let config = LokiConfig::from_values(Some(" https://loki.sebasgc.xyz "), None, None)
            .unwrap()
            .expect("configured Loki");

        assert_eq!(
            config.push_url.as_str(),
            "https://loki.sebasgc.xyz/loki/api/v1/push"
        );
        assert_eq!(config.service, "litepod");
        assert_eq!(config.environment, "dev");
    }
}
