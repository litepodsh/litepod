use axum::{Router, extract::State, response::IntoResponse, routing::get};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::billing::repository;
use crate::dtos::deployment::LiveRouteDto;
use crate::dtos::response::{AppError, ok};
use crate::state::state::SharedState;

fn read_mem_available_kb() -> Option<u64> {
    let content = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("MemAvailable:") {
            let kb: u64 = rest.trim().split_whitespace().next()?.parse().ok()?;
            return Some(kb);
        }
    }
    None
}

pub fn routes() -> Router<SharedState> {
    Router::new()
        .route("/api/v1/debug/overview", get(debug_overview_handler))
        .route("/api/v1/debug/billing", get(billing_status_handler))
        .route("/api/v1/debug/database", get(database_storage_handler))
    // Podman management moved to the per-server console under
    // `/api/v1/organizations/{org}/servers/{server_id}/podman/...`
    // (see `handler::podman_console`). Caddy management likewise moved to
    // `/api/v1/organizations/{org}/servers/{server_id}/caddy/...`
    // (see `handler::caddy_console`).
}

#[derive(Serialize)]
struct ServiceStatus {
    status: &'static str,
    detail: Value,
}

#[derive(Serialize)]
struct BillingDebugEvent {
    event_id: String,
    event_type: String,
    occurred_at: i64,
    outcome: String,
    last_error: Option<String>,
}

#[derive(Serialize)]
struct BillingDebugQuantitySyncAttempt {
    user_id: String,
    polar_subscription_id: String,
    previous_quantity: i64,
    target_quantity: i64,
    proration_mode: String,
    status: String,
    error_message: Option<String>,
    attempted_at: i64,
}

#[derive(Serialize)]
struct BillingDebugStatus {
    configured: bool,
    environment: Option<&'static str>,
    access_token_configured: bool,
    webhook_secret_configured: bool,
    events: Vec<BillingDebugEvent>,
    deliveries: Vec<repository::WebhookDeliveryRow>,
    quantity_sync_attempts: Vec<BillingDebugQuantitySyncAttempt>,
}

async fn billing_status_handler(
    State(state): State<SharedState>,
) -> Result<impl IntoResponse, AppError> {
    let config = state.polar.as_ref();
    use sqlx::Row;
    let conn = crate::git::repositories::db::get_conn(&state).await;
    let rows = sqlx::query("SELECT polar_event_id, event_type, occurred_at, outcome, last_error FROM billing_events ORDER BY received_at DESC LIMIT 25")
        .fetch_all(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut events = Vec::new();
    for row in &rows {
        events.push(BillingDebugEvent {
            event_id: row
                .try_get(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            event_type: row
                .try_get(1)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            occurred_at: row
                .try_get(2)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            outcome: row
                .try_get(3)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            last_error: row
                .try_get(4)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        });
    }
    let deliveries = repository::recent_webhook_deliveries(&state, 25).await?;
    let quantity_sync_attempts = repository::recent_quantity_sync_attempts(&state, 25)
        .await?
        .into_iter()
        .map(
            |(
                user_id,
                polar_subscription_id,
                previous_quantity,
                target_quantity,
                proration_mode,
                status,
                error_message,
                attempted_at,
            )| BillingDebugQuantitySyncAttempt {
                user_id,
                polar_subscription_id,
                previous_quantity,
                target_quantity,
                proration_mode,
                status,
                error_message,
                attempted_at,
            },
        )
        .collect();
    Ok(ok(BillingDebugStatus {
        configured: config.is_some(),
        environment: config.map(|value| value.environment.as_str()),
        access_token_configured: config.is_some_and(|value| !value.access_token.is_empty()),
        webhook_secret_configured: config.is_some_and(|value| !value.webhook_secret.is_empty()),
        events,
        deliveries,
        quantity_sync_attempts,
    }))
}

pub(crate) fn redact_value(value: &mut Value) {
    const SECRET_MARKERS: &[&str] = &[
        "password",
        "passwd",
        "secret",
        "token",
        "apikey",
        "api_key",
        "authorization",
        "credential",
        "private_key",
    ];
    fn secret_key(key: &str) -> bool {
        key.eq_ignore_ascii_case("key")
            || SECRET_MARKERS
                .iter()
                .any(|marker| key.to_ascii_lowercase().contains(marker))
    }
    match value {
        Value::Object(object) => {
            for (key, child) in object.iter_mut() {
                if secret_key(key) {
                    *child = Value::String("[REDACTED]".to_string());
                } else {
                    if matches!(key.as_str(), "Env" | "env" | "Environment" | "environment") {
                        if let Value::Array(values) = child {
                            for item in values.iter_mut() {
                                if let Value::String(variable) = item {
                                    if let Some((name, _)) = variable.split_once('=') {
                                        if secret_key(name) {
                                            *variable = format!("{name}=[REDACTED]");
                                        }
                                    }
                                }
                            }
                        }
                    }
                    redact_value(child);
                }
            }
        }
        Value::Array(values) => values.iter_mut().for_each(redact_value),
        _ => {}
    }
}

fn safe_json<T: Serialize>(value: T) -> Result<Value, AppError> {
    let mut value =
        serde_json::to_value(value).map_err(|error| AppError::InternalError(error.to_string()))?;
    redact_value(&mut value);
    Ok(value)
}

use crate::podman::podman_socket::podman_client as podman;

pub(crate) async fn caddy_live_config(state: &SharedState) -> Result<Value, AppError> {
    let (client, base) = crate::deployment::caddy_admin::client(&state.caddy_admin_url, None)
        .map_err(AppError::InternalError)?;
    let response = client
        .get(format!("{base}/config/"))
        .send()
        .await
        .map_err(|error| AppError::InternalError(format!("Caddy is unavailable: {error}")))?;
    if !response.status().is_success() {
        return Err(AppError::InternalError(format!(
            "Caddy returned {}",
            response.status()
        )));
    }
    let mut config: Value = response
        .json()
        .await
        .map_err(|error| AppError::InternalError(format!("Invalid Caddy response: {error}")))?;
    redact_value(&mut config);
    Ok(config)
}

/// The routes Caddy is actually serving, flattened out of `srv0`.
///
/// Read straight from the admin API rather than from what litepod believes it
/// pushed, because Caddy holds its configuration in memory: a restart silently
/// drops everything, and only a read-back can show that.
pub(crate) async fn caddy_live_routes(state: &SharedState) -> Result<Vec<LiveRouteDto>, AppError> {
    let config = caddy_live_config(state).await?;
    let routes = config
        .pointer("/apps/http/servers/srv0/routes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(routes
        .iter()
        .map(|route| LiveRouteDto {
            route_id: route.get("@id").and_then(Value::as_str).map(str::to_owned),
            hosts: route
                .get("match")
                .and_then(Value::as_array)
                .map(|matchers| {
                    matchers
                        .iter()
                        .filter_map(|matcher| matcher.get("host").and_then(Value::as_array))
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            dial: route
                .pointer("/handle/0/upstreams/0/dial")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
        .collect())
}

pub async fn debug_overview_handler(
    State(state): State<SharedState>,
) -> Result<impl IntoResponse, AppError> {
    let caddy_check = async {
        Ok::<ServiceStatus, AppError>(match caddy_live_config(&state).await {
            Ok(config) => ServiceStatus {
                status: "up",
                detail: json!({ "config": config }),
            },
            Err(error) => ServiceStatus {
                status: "down",
                detail: json!({ "message": error.to_string() }),
            },
        })
    };
    let podman_check = async {
        Ok::<ServiceStatus, AppError>(match podman(&state).and_then(|client| Ok(client)) {
            Ok(client) => {
                let (info, version) = tokio::join!(client.info(), client.version());
                match (info, version) {
                    (Ok(info), Ok(version)) => {
                        let mut detail =
                            json!({ "info": safe_json(info)?, "version": safe_json(version)? });
                        if let Some(mem_available_kb) = read_mem_available_kb() {
                            if let Some(host) =
                                detail.get_mut("info").and_then(|i| i.get_mut("host"))
                            {
                                host["memAvailable"] = json!(mem_available_kb * 1024);
                            }
                        }
                        ServiceStatus {
                            status: "up",
                            detail,
                        }
                    }
                    (Err(error), _) | (_, Err(error)) => ServiceStatus {
                        status: "down",
                        detail: json!({ "message": error.to_string() }),
                    },
                }
            }
            Err(error) => ServiceStatus {
                status: "down",
                detail: json!({ "message": error.to_string() }),
            },
        })
    };
    let (caddy, podman_status) = tokio::join!(caddy_check, podman_check);
    Ok(ok(json!({ "caddy": caddy?, "podman": podman_status? })))
}

#[derive(Serialize, Debug, PartialEq)]
struct TableUsage {
    name: String,
    /// Pages owned by the table and its indexes, in bytes (`per_table_mode = "bytes"`).
    bytes: Option<i64>,
    /// Row count, only when `dbstat` is unavailable (`per_table_mode = "rows"`).
    rows: Option<i64>,
}

#[derive(Serialize)]
struct DatabaseStorage {
    file_bytes: Option<u64>,
    wal_bytes: Option<u64>,
    shm_bytes: Option<u64>,
    page_size: Option<i64>,
    page_count: Option<i64>,
    freelist_count: Option<i64>,
    /// `page_size * freelist_count`: space a VACUUM would give back.
    free_bytes: Option<i64>,
    per_table_mode: &'static str,
    tables: Vec<TableUsage>,
}

/// The DuckDB metrics store: its stats, or why they could not be read. Kept apart so a
/// broken metrics store never hides the SQLite report.
#[derive(Serialize)]
struct MetricsStorage {
    stats: Option<crate::metrics_history::DebugStats>,
    error: Option<String>,
}

#[derive(Serialize)]
struct DragonflyStorage {
    stats: Option<crate::scheduled_tasks::ephemeral::DebugStats>,
    error: Option<String>,
}

#[derive(Serialize)]
struct StorageReport {
    #[serde(flatten)]
    sqlite: DatabaseStorage,
    metrics: MetricsStorage,
    dragonfly: DragonflyStorage,
}

/// Read-only storage report for the litepod SQLite database. Never returns row contents:
/// only sizes, page counters and per-table totals.
async fn database_storage_handler(
    State(state): State<SharedState>,
) -> Result<impl IntoResponse, AppError> {
    let conn = crate::git::repositories::db::get_conn(&state).await;
    let (sqlite, metrics, dragonfly) = tokio::join!(
        database_storage(&conn),
        crate::metrics_history::debug_stats(),
        state.ephemeral.debug_stats()
    );
    let metrics = match metrics {
        Ok(stats) => MetricsStorage {
            stats: Some(stats),
            error: None,
        },
        Err(error) => MetricsStorage {
            stats: None,
            error: Some(error.to_string()),
        },
    };
    let dragonfly = match dragonfly {
        Ok(stats) => DragonflyStorage {
            stats: Some(stats),
            error: None,
        },
        Err(error) => DragonflyStorage {
            stats: None,
            error: Some(error),
        },
    };
    Ok(ok(StorageReport {
        sqlite,
        metrics,
        dragonfly,
    }))
}

async fn database_storage(conn: &SqlitePool) -> DatabaseStorage {
    let pragma = |sql: &'static str| async move {
        sqlx::query_scalar::<_, i64>(sql).fetch_one(conn).await.ok()
    };
    let page_size = pragma("PRAGMA page_size").await;
    let page_count = pragma("PRAGMA page_count").await;
    let freelist_count = pragma("PRAGMA freelist_count").await;
    let path: Option<String> =
        sqlx::query_scalar("SELECT file FROM pragma_database_list WHERE name = 'main'")
            .fetch_optional(conn)
            .await
            .ok()
            .flatten()
            .filter(|path: &String| !path.is_empty());
    let size = |path: &std::path::Path| std::fs::metadata(path).ok().map(|m| m.len());
    let sibling = |suffix: &str| {
        path.as_deref()
            .and_then(|path| size(std::path::Path::new(&format!("{path}{suffix}"))))
    };
    let (per_table_mode, tables) = table_usage(conn).await;
    DatabaseStorage {
        file_bytes: sibling(""),
        wal_bytes: sibling("-wal"),
        shm_bytes: sibling("-shm"),
        page_size,
        page_count,
        freelist_count,
        free_bytes: page_size
            .zip(freelist_count)
            .map(|(size, count)| size * count),
        per_table_mode,
        tables,
    }
}

/// Bytes per table from `dbstat` (indexes folded into their table), or row counts when this
/// SQLite build has no `dbstat`. Largest first.
async fn table_usage(conn: &SqlitePool) -> (&'static str, Vec<TableUsage>) {
    match table_bytes(conn).await {
        Ok(tables) => ("bytes", tables),
        Err(_) => ("rows", table_rows(conn).await),
    }
}

async fn table_bytes(conn: &SqlitePool) -> Result<Vec<TableUsage>, sqlx::Error> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT COALESCE(m.tbl_name, d.name) AS name, SUM(d.pgsize) AS bytes \
         FROM dbstat d LEFT JOIN sqlite_master m ON m.name = d.name \
         GROUP BY 1 ORDER BY bytes DESC, name",
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(name, bytes)| TableUsage {
            name,
            bytes: Some(bytes),
            rows: None,
        })
        .collect())
}

async fn table_rows(conn: &SqlitePool) -> Vec<TableUsage> {
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_all(conn)
    .await
    .unwrap_or_default();
    let mut tables = Vec::with_capacity(names.len());
    for name in names {
        // the name comes from sqlite_master, quoted as an identifier; only the count is read
        let sql = format!("SELECT COUNT(*) FROM \"{}\"", name.replace('"', "\"\""));
        let rows = sqlx::query_scalar::<_, i64>(&sql)
            .fetch_one(conn)
            .await
            .ok();
        tables.push(TableUsage {
            name,
            bytes: None,
            rows,
        });
    }
    tables.sort_by(|a, b| b.rows.cmp(&a.rows).then_with(|| a.name.cmp(&b.name)));
    tables
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{TableUsage, redact_value, table_rows, table_usage};

    async fn pool_with_tables() -> sqlx::SqlitePool {
        // one connection: every in-memory connection would otherwise be its own database
        let conn = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for sql in [
            "CREATE TABLE big (id INTEGER PRIMARY KEY, body TEXT NOT NULL)",
            "CREATE INDEX idx_big_body ON big(body)",
            "CREATE TABLE small (id TEXT PRIMARY KEY)",
            "INSERT INTO small VALUES ('a')",
        ] {
            sqlx::query(sql).execute(&conn).await.unwrap();
        }
        for i in 0..200 {
            sqlx::query("INSERT INTO big (body) VALUES (?)")
                .bind(format!("{i}-{}", "x".repeat(400)))
                .execute(&conn)
                .await
                .unwrap();
        }
        conn
    }

    #[tokio::test]
    async fn table_usage_folds_indexes_into_their_table_largest_first() {
        let conn = pool_with_tables().await;
        let (mode, tables) = table_usage(&conn).await;
        let names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
        if mode == "bytes" {
            assert_eq!(names.first(), Some(&"big"));
            assert!(!names.contains(&"idx_big_body"), "{names:?}");
            assert!(!names.contains(&"sqlite_autoindex_small_1"), "{names:?}");
            assert!(tables.iter().all(|t| t.bytes.is_some() && t.rows.is_none()));
        } else {
            assert_eq!(mode, "rows");
            assert_eq!(names, ["big", "small"]);
        }
    }

    #[tokio::test]
    async fn row_fallback_counts_rows_without_reading_them() {
        let conn = pool_with_tables().await;
        assert_eq!(
            table_rows(&conn).await,
            vec![
                TableUsage {
                    name: "big".into(),
                    bytes: None,
                    rows: Some(200),
                },
                TableUsage {
                    name: "small".into(),
                    bytes: None,
                    rows: Some(1),
                },
            ]
        );
    }

    #[test]
    fn redacts_nested_sensitive_values_without_hiding_regular_data() {
        let mut value = json!({
            "Name": "demo",
            "Config": { "Env": ["API_TOKEN=visible-to-daemon", "PORT=6001"] },
            "items": [{ "password": "nope" }]
        });
        redact_value(&mut value);

        assert_eq!(value["Name"], "demo");
        assert_eq!(value["Config"]["Env"][0], "API_TOKEN=[REDACTED]");
        assert_eq!(value["Config"]["Env"][1], "PORT=6001");
        assert_eq!(value["items"][0]["password"], "[REDACTED]");
    }
}
