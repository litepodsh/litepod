use std::{collections::BTreeSet, ffi::OsString};

use axum::{
    extract::{Path, State},
    response::IntoResponse,
};
use chrono::Utc;
use serde_json::Value;

use crate::{
    deployment::network_observability::parse_net_io,
    dtos::{
        deployment::{ApplicationNetworkStatsDto, ContainerNetStatDto},
        response::{AppError, ok},
    },
    security::authz::extract::{RequireApp, perm},
    servers::runtime::ServerRuntime,
    state::state::SharedState,
};

type ApplicationPath = (String, String, String, String);

fn field<'a>(object: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| object.get(key).and_then(Value::as_str))
}

pub async fn application_network_stats_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
    Path(_): Path<ApplicationPath>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = &app.application_id;
    let runtime = ServerRuntime::for_application(&state, application_id).await?;
    let owned = crate::handler::deployments::container_names(&state, application_id)
        .await?
        .into_iter()
        .collect::<BTreeSet<_>>();
    if owned.is_empty() {
        return Ok(ok(ApplicationNetworkStatsDto {
            containers: Vec::new(),
            observed_at: Utc::now().to_rfc3339(),
        }));
    }

    let output = runtime
        .podman(&[
            OsString::from("stats"),
            OsString::from("--no-stream"),
            OsString::from("--format"),
            OsString::from("json"),
        ])
        .await?;
    let rows: Vec<Value> = serde_json::from_str(output.stdout.trim()).map_err(|error| {
        AppError::InternalError(format!("podman stats returned invalid JSON: {error}"))
    })?;

    let mut containers = rows
        .iter()
        .filter_map(|row| {
            let name = field(row, &["Name", "name"])?;
            if !owned.contains(name) {
                return None;
            }
            let (rx, tx) = field(row, &["NetIO", "net_io"])
                .map(parse_net_io)
                .unwrap_or((0, 0));
            Some(ContainerNetStatDto {
                container: name.to_string(),
                net_rx_bytes: rx,
                net_tx_bytes: tx,
            })
        })
        .collect::<Vec<_>>();
    containers.sort_by(|left, right| left.container.cmp(&right.container));

    Ok(ok(ApplicationNetworkStatsDto {
        containers,
        observed_at: Utc::now().to_rfc3339(),
    }))
}
