use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
};

use crate::{dtos::response::AppError, servers::runtime::ServerRuntime};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerMetricState {
    Running,
    NotRunning,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContainerMetric {
    pub id: String,
    pub name: String,
    pub state: ContainerMetricState,
    pub cpu_percent: Option<f64>,
    pub memory_usage_bytes: Option<u64>,
    pub memory_limit_bytes: Option<u64>,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContainerMetricsSnapshot {
    pub observed_at: DateTime<Utc>,
    pub metrics: Vec<ContainerMetric>,
}

#[derive(Debug, Clone)]
pub(crate) struct ContainerIdentity {
    id: String,
    name: String,
    state: ContainerMetricState,
}

impl ContainerIdentity {
    pub(crate) fn running(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            state: ContainerMetricState::Running,
        }
    }

    #[cfg(test)]
    fn stopped(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            state: ContainerMetricState::NotRunning,
        }
    }

    fn unavailable(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            state: ContainerMetricState::Unavailable,
        }
    }
}

/// Collects stats through an application's resolved runtime. Unlike the debug
/// collector above, this path never opens the control-plane Podman socket for
/// an application assigned to an SSH server.
pub(crate) async fn collect_runtime_container_metrics(
    runtime: &ServerRuntime,
    filters: &[String],
) -> Result<ContainerMetricsSnapshot, AppError> {
    let output = runtime
        .podman(&[
            OsString::from("ps"),
            OsString::from("--all"),
            OsString::from("--format"),
            OsString::from("json"),
        ])
        .await?;
    let containers: Value = serde_json::from_str(&output.stdout).map_err(|error| {
        AppError::InternalError(format!("Podman ps returned invalid JSON: {error}"))
    })?;
    let identities = discover_cli_identities(&containers, filters);
    let running_names = identities
        .iter()
        .filter(|identity| identity.state == ContainerMetricState::Running)
        .map(|identity| OsString::from(&identity.name))
        .collect::<Vec<_>>();
    let stats = if running_names.is_empty() {
        Value::Array(Vec::new())
    } else {
        let mut args = vec![
            OsString::from("stats"),
            OsString::from("--no-stream"),
            OsString::from("--format"),
            OsString::from("json"),
        ];
        args.extend(running_names);
        runtime
            .podman(&args)
            .await
            .ok()
            .and_then(|output| serde_json::from_str(&output.stdout).ok())
            .unwrap_or(Value::Array(Vec::new()))
    };
    Ok(normalize_stats(stats, &identities, Utc::now()))
}

fn discover_cli_identities(containers: &Value, filters: &[String]) -> Vec<ContainerIdentity> {
    let mut matched_filters = HashSet::new();
    let mut identities = Vec::new();
    for container in containers.as_array().into_iter().flatten() {
        let Some(id) = container
            .get("Id")
            .or_else(|| container.get("ID"))
            .or_else(|| container.get("id"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let name = container
            .get("Names")
            .or_else(|| container.get("Name"))
            .or_else(|| container.get("name"))
            .and_then(|value| match value {
                Value::String(name) => Some(name.clone()),
                Value::Array(names) => names.first().and_then(Value::as_str).map(str::to_owned),
                _ => None,
            })
            .unwrap_or_else(|| id.to_string());
        let name = name.trim_start_matches('/').to_string();
        let matches_filter = filters.is_empty()
            || filters.iter().any(|filter| {
                let matches = filter == id || filter == &name;
                if matches {
                    matched_filters.insert(filter.clone());
                }
                matches
            });
        if !matches_filter {
            continue;
        }
        let running = container
            .get("State")
            .or_else(|| container.get("state"))
            .and_then(Value::as_str)
            .is_some_and(|state| state.eq_ignore_ascii_case("running"));
        identities.push(ContainerIdentity {
            id: id.to_string(),
            name,
            state: if running {
                ContainerMetricState::Running
            } else {
                ContainerMetricState::NotRunning
            },
        });
    }
    for filter in filters {
        if !matched_filters.contains(filter) {
            identities.push(ContainerIdentity::unavailable(filter, filter));
        }
    }
    identities
}

pub(crate) fn normalize_stats(
    stats: Value,
    identities: &[ContainerIdentity],
    observed_at: DateTime<Utc>,
) -> ContainerMetricsSnapshot {
    let mut values = Vec::new();
    collect_stat_values(&stats, None, &mut values);
    let mut by_identifier = HashMap::new();
    for (map_key, value) in values {
        if let Some(map_key) = map_key {
            by_identifier.insert(map_key.trim_start_matches('/').to_string(), value);
        }
        for field in [
            "ContainerID",
            "container_id",
            "ID",
            "Id",
            "id",
            "Name",
            "name",
            "ContainerName",
            "container_name",
        ] {
            if let Some(identifier) = value.get(field).and_then(Value::as_str) {
                by_identifier.insert(identifier.trim_start_matches('/').to_string(), value);
            }
        }
    }

    let metrics = identities
        .iter()
        .map(|identity| {
            let stat = by_identifier
                .get(&identity.id)
                .copied()
                .or_else(|| by_identifier.get(&identity.name).copied())
                .or_else(|| {
                    by_identifier.iter().find_map(|(identifier, value)| {
                        (identity.id.starts_with(identifier)
                            || identifier.starts_with(&identity.id))
                        .then_some(*value)
                    })
                });
            let state = match (&identity.state, stat) {
                (ContainerMetricState::Running, None) => ContainerMetricState::Unavailable,
                (state, _) => state.clone(),
            };
            let (memory_usage_bytes, memory_limit_bytes) = stat
                .map(|value| memory_values(value))
                .unwrap_or((None, None));
            ContainerMetric {
                id: identity.id.clone(),
                name: identity.name.clone(),
                state,
                cpu_percent: stat.and_then(|value| number(value, "CPU", "cpu")),
                memory_usage_bytes,
                memory_limit_bytes,
                observed_at,
            }
        })
        .collect();

    ContainerMetricsSnapshot {
        observed_at,
        metrics,
    }
}

fn collect_stat_values<'a>(
    value: &'a Value,
    map_key: Option<&'a str>,
    output: &mut Vec<(Option<&'a str>, &'a Value)>,
) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_stat_values(value, None, output);
            }
        }
        Value::Object(values) if is_stat_value(values) => output.push((map_key, value)),
        Value::Object(values) => {
            for (key, value) in values {
                collect_stat_values(value, Some(key), output);
            }
        }
        _ => {}
    }
}

fn is_stat_value(values: &serde_json::Map<String, Value>) -> bool {
    [
        "ContainerID",
        "container_id",
        "ID",
        "Id",
        "id",
        "Name",
        "name",
        "CPU",
        "cpu",
        "cpu_percent",
        "MemUsage",
        "mem_usage",
    ]
    .iter()
    .any(|field| values.contains_key(*field))
}

fn number(value: &Value, primary: &str, alternative: &str) -> Option<f64> {
    let raw = value
        .get(primary)
        .or_else(|| value.get(alternative))
        .or_else(|| value.get("cpu_percent"))?;
    raw.as_f64().or_else(|| {
        raw.as_str()
            .and_then(|value| value.trim().trim_end_matches('%').trim().parse().ok())
    })
}

fn memory_values(value: &Value) -> (Option<u64>, Option<u64>) {
    let used = value.get("MemUsage").or_else(|| value.get("mem_usage"));
    let mut used_bytes = used.and_then(value_as_bytes);
    let mut limit_bytes = value
        .get("MemLimit")
        .or_else(|| value.get("mem_limit"))
        .and_then(value_as_bytes);

    if let Some(memory) = used.and_then(Value::as_str) {
        if let Some((used, limit)) = memory.split_once('/') {
            used_bytes = parse_bytes(used);
            limit_bytes = parse_bytes(limit).or(limit_bytes);
        }
    }

    (used_bytes, limit_bytes)
}

fn value_as_bytes(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(parse_bytes))
}

fn parse_bytes(value: &str) -> Option<u64> {
    let value = value.trim();
    if let Ok(bytes) = value.parse() {
        return Some(bytes);
    }
    let split = value.find(|character: char| character.is_ascii_alphabetic())?;
    let amount: f64 = value[..split].trim().parse().ok()?;
    let unit = value[split..].trim().to_ascii_lowercase();
    let multiplier = match unit.as_str() {
        "b" => 1.0,
        "kb" => 1_000.0,
        "mb" => 1_000_000.0,
        "gb" => 1_000_000_000.0,
        "tb" => 1_000_000_000_000.0,
        "kib" => 1_024.0,
        "mib" => 1_048_576.0,
        "gib" => 1_073_741_824.0,
        "tib" => 1_099_511_627_776.0,
        _ => return None,
    };
    (amount * multiplier)
        .round()
        .is_finite()
        .then(|| (amount * multiplier).round() as u64)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    use super::{
        ContainerIdentity, ContainerMetricState, discover_cli_identities, normalize_stats,
    };

    fn fixed_time() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 20, 12, 0, 0).unwrap()
    }

    #[test]
    fn normalizes_podman_cpu_and_memory_fields() {
        let snapshot = normalize_stats(
            json!([{
                "ContainerID": "abc123",
                "Name": "demo-api",
                "CPU": 12.5,
                "MemUsage": 67_108_864,
                "MemLimit": 268_435_456
            }]),
            &[ContainerIdentity::running("abc123", "demo-api")],
            fixed_time(),
        );

        assert_eq!(snapshot.metrics[0].cpu_percent, Some(12.5));
        assert_eq!(snapshot.metrics[0].memory_usage_bytes, Some(67_108_864));
        assert_eq!(snapshot.metrics[0].memory_limit_bytes, Some(268_435_456));
    }

    #[test]
    fn reports_requested_non_running_containers_without_stale_numbers() {
        let snapshot = normalize_stats(
            json!([]),
            &[ContainerIdentity::stopped("old", "old")],
            fixed_time(),
        );

        assert_eq!(snapshot.metrics[0].state, ContainerMetricState::NotRunning);
        assert_eq!(snapshot.metrics[0].cpu_percent, None);
    }

    #[test]
    fn normalizes_human_readable_podman_stats_fields() {
        let snapshot = normalize_stats(
            json!([{
                "ContainerID": "abc123",
                "Name": "demo-api",
                "CPU": "12.5%",
                "MemUsage": "64MiB / 256MiB"
            }]),
            &[ContainerIdentity::running("abc123", "demo-api")],
            fixed_time(),
        );

        assert_eq!(snapshot.metrics[0].cpu_percent, Some(12.5));
        assert_eq!(snapshot.metrics[0].memory_usage_bytes, Some(67_108_864));
        assert_eq!(snapshot.metrics[0].memory_limit_bytes, Some(268_435_456));
    }

    #[test]
    fn cli_identities_only_return_requested_application_containers() {
        let identities = discover_cli_identities(
            &json!([
                { "Id": "app-id", "Names": ["/application-web"], "State": "running" },
                { "Id": "other-id", "Names": ["/unrelated"], "State": "running" }
            ]),
            &["application-web".into(), "removed-worker".into()],
        );

        assert_eq!(identities.len(), 2);
        assert_eq!(identities[0].id, "app-id");
        assert_eq!(identities[0].name, "application-web");
        assert_eq!(identities[0].state, ContainerMetricState::Running);
        assert_eq!(identities[1].id, "removed-worker");
        assert_eq!(identities[1].state, ContainerMetricState::Unavailable);
    }

    #[test]
    fn cli_identities_include_every_container_without_filters() {
        let identities = discover_cli_identities(
            &json!([
                { "Id": "web-id", "Names": ["/web"], "State": "running" },
                { "Id": "db-id", "Names": ["/db"], "State": "exited" }
            ]),
            &[],
        );

        assert_eq!(identities.len(), 2);
        assert_eq!(identities[0].name, "web");
        assert_eq!(identities[1].state, ContainerMetricState::NotRunning);
    }

    #[test]
    fn normalizes_podman_cli_json_with_short_id_and_snake_case_fields() {
        let snapshot = normalize_stats(
            json!([{
                "id": "3fa142773664",
                "name": "bmxdocs-prod-websocket-ed3fa4",
                "cpu_percent": "0.31%",
                "mem_usage": "40.55MB / 8.29GB"
            }]),
            &[ContainerIdentity::running(
                "3fa14277366461b639f7cc58b1fe43876a16c3b9a6506aeeb5ef888eb349d3b4",
                "bmxdocs-prod-websocket-ed3fa4",
            )],
            fixed_time(),
        );

        assert_eq!(snapshot.metrics[0].state, ContainerMetricState::Running);
        assert_eq!(snapshot.metrics[0].cpu_percent, Some(0.31));
        assert_eq!(snapshot.metrics[0].memory_usage_bytes, Some(40_550_000));
        assert_eq!(snapshot.metrics[0].memory_limit_bytes, Some(8_290_000_000));
    }

    #[test]
    fn normalizes_stats_nested_in_a_container_id_map() {
        let snapshot = normalize_stats(
            json!({
                "3fa142773664": {
                    "cpu_percent": "0.31%",
                    "mem_usage": "40.55MB / 8.29GB"
                }
            }),
            &[ContainerIdentity::running(
                "3fa14277366461b639f7cc58b1fe43876a16c3b9a6506aeeb5ef888eb349d3b4",
                "bmxdocs-prod-websocket-ed3fa4",
            )],
            fixed_time(),
        );

        assert_eq!(snapshot.metrics[0].state, ContainerMetricState::Running);
        assert_eq!(snapshot.metrics[0].cpu_percent, Some(0.31));
    }
}
