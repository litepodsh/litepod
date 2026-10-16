use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
};

use axum::{
    extract::{Path, State},
    response::IntoResponse,
};
use chrono::Utc;
use serde_json::Value;

use crate::{
    deployment::{compose::proxy_routes, network_observability, source},
    dtos::{
        deployment::{ApplicationNetworkProbeDto, NetworkProbePairDto},
        response::{AppError, ok},
    },
    security::authz::extract::{RequireApp, perm},
    servers::runtime::ServerRuntime,
    state::state::SharedState,
};

type ApplicationPath = (String, String, String, String);

/// Runs inside the source container. Always exits 0 and prints one token so a
/// closed port is a result to classify, not a command failure.
const PROBE: &str = r#"h=$1; p=$2;
if command -v nc >/dev/null 2>&1 && [ -n "$p" ]; then
  o=$(nc -z -w2 "$h" "$p" 2>&1); if [ $? -eq 0 ]; then echo PORTOK; else echo "PORTERR:$o"; fi; exit 0;
fi;
if command -v getent >/dev/null 2>&1; then getent hosts "$h" >/dev/null 2>&1 && echo DNSOK || echo DNSERR; exit 0; fi;
if command -v nslookup >/dev/null 2>&1; then nslookup "$h" >/dev/null 2>&1 && echo DNSOK || echo DNSERR; exit 0; fi;
if command -v ping >/dev/null 2>&1; then ping -c1 -W1 "$h" >/dev/null 2>&1 && echo DNSOK || echo DNSERR; exit 0; fi;
echo NOTOOLS"#;

fn inspected_object(output: &str) -> Result<Value, AppError> {
    let value: Value = serde_json::from_str(output).map_err(|error| {
        AppError::InternalError(format!("Podman inspect returned invalid JSON: {error}"))
    })?;
    Ok(value
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .unwrap_or(value))
}

pub async fn application_network_probe_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
    Path(_): Path<ApplicationPath>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = &app.application_id;
    let source = source::get(&state, application_id).await?;
    // Peer probing only makes sense for a multi-service Compose app.
    if source.deploy_type != "compose" {
        return Ok(ok(ApplicationNetworkProbeDto {
            pairs: Vec::new(),
            observed_at: Utc::now().to_rfc3339(),
        }));
    }
    let runtime = ServerRuntime::for_application(&state, application_id).await?;

    let service_by_container =
        crate::deployment::compose_standalone::status(&state, application_id)
            .await?
            .into_iter()
            .map(|service| (service.container_name, service.service))
            .collect::<BTreeMap<_, _>>();
    let port_by_service = proxy_routes::list(&state, application_id)
        .await?
        .into_iter()
        .map(|route| (route.service, route.container_port))
        .collect::<BTreeMap<_, _>>();

    // container name -> its private (non-proxy) network names
    let mut private_nets: Vec<(String, BTreeSet<String>)> = Vec::new();
    for name in crate::handler::deployments::container_names(&state, application_id).await? {
        let output = match runtime
            .podman(&[OsString::from("inspect"), OsString::from(&name)])
            .await
        {
            Ok(output) => output,
            Err(error) if crate::handler::deployments::missing_podman_resource(&error) => {
                continue;
            }
            Err(error) => return Err(error),
        };
        let inspect = inspected_object(&output.stdout)?;
        let nets = network_observability::container_networks(&inspect)
            .into_iter()
            .map(|attachment| attachment.name)
            .filter(|net| net != &state.podman_network)
            .collect::<BTreeSet<_>>();
        private_nets.push((name, nets));
    }
    private_nets.sort_by(|left, right| left.0.cmp(&right.0));

    let mut pairs = Vec::new();
    for window in private_nets.windows(2) {
        let (from, from_nets) = &window[0];
        let (to, to_nets) = &window[1];
        if from_nets.is_disjoint(to_nets) {
            continue; // no shared private network -> the missing peer edge already says so
        }
        let to_service = service_by_container.get(to).cloned();
        let from_service = service_by_container.get(from).cloned();
        let host = to_service.clone().unwrap_or_else(|| to.clone());
        let port = to_service
            .as_ref()
            .and_then(|s| port_by_service.get(s).copied());
        let port_arg = port.map(|p| p.to_string()).unwrap_or_default();

        let reason = match runtime
            .podman(&[
                OsString::from("exec"),
                OsString::from(from),
                OsString::from("sh"),
                OsString::from("-c"),
                OsString::from(PROBE),
                OsString::from("_"),
                OsString::from(&host),
                OsString::from(&port_arg),
            ])
            .await
        {
            Ok(output) => network_observability::classify_probe(&output.stdout).to_string(),
            Err(_) => "probe_failed".to_string(),
        };

        pairs.push(NetworkProbePairDto {
            from: from.clone(),
            to: to.clone(),
            from_service,
            to_service,
            port,
            reason,
        });
    }

    Ok(ok(ApplicationNetworkProbeDto {
        pairs,
        observed_at: Utc::now().to_rfc3339(),
    }))
}
