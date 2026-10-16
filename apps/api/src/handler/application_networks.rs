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
    deployment::{
        compose::proxy_routes,
        network_observability::{self, ObservedContainer},
        resource_networks, source,
    },
    dtos::{
        deployment::ApplicationNetworkOverviewDto,
        response::{AppError, ok},
    },
    security::authz::extract::{RequireApp, perm},
    servers::runtime::ServerRuntime,
    state::state::SharedState,
};

type ApplicationPath = (String, String, String, String);
type ResourcePath = (String, String, String, String, String);

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

use crate::handler::deployments::missing_podman_resource as missing_resource;

pub async fn application_networks_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
    Path(_): Path<ApplicationPath>,
) -> Result<impl IntoResponse, AppError> {
    let application_id = &app.application_id;
    let runtime = ServerRuntime::for_application(&state, application_id).await?;
    let source = source::get(&state, application_id).await?;
    let is_compose = source.deploy_type == "compose";
    let service_names = if is_compose {
        crate::deployment::compose_standalone::status(&state, application_id)
            .await?
            .into_iter()
            .map(|service| (service.container_name, service.service))
            .collect::<BTreeMap<_, _>>()
    } else {
        BTreeMap::new()
    };
    let mut containers = Vec::new();
    for name in crate::handler::deployments::container_names(&state, application_id).await? {
        let output = match runtime
            .podman(&[OsString::from("inspect"), OsString::from(&name)])
            .await
        {
            Ok(output) => output,
            Err(error) if missing_resource(&error) => continue,
            Err(error) => return Err(error),
        };
        let inspect = inspected_object(&output.stdout)?;
        containers.push(ObservedContainer {
            service: service_names.get(&name).cloned(),
            networks: network_observability::container_networks(&inspect),
            name,
        });
    }
    let network_names = containers
        .iter()
        .flat_map(|container| {
            container
                .networks
                .iter()
                .map(|network| network.name.clone())
        })
        .collect::<BTreeSet<_>>();
    let mut inspected_networks = BTreeMap::new();
    for name in network_names {
        let output = match runtime
            .podman(&[
                OsString::from("network"),
                OsString::from("inspect"),
                OsString::from(&name),
            ])
            .await
        {
            Ok(output) => output,
            Err(error) if missing_resource(&error) => continue,
            Err(error) => return Err(error),
        };
        inspected_networks.insert(
            name,
            network_observability::network_members(&inspected_object(&output.stdout)?),
        );
    }
    let proxy_services = if is_compose {
        proxy_routes::list(&state, application_id)
            .await?
            .into_iter()
            .map(|route| route.service)
            .collect()
    } else {
        BTreeSet::new()
    };
    let (containers, networks, findings) = network_observability::summarize(
        containers,
        inspected_networks,
        &proxy_services,
        &state.podman_network,
        runtime.is_remote(),
        is_compose,
    );
    Ok(ok(ApplicationNetworkOverviewDto {
        containers,
        networks,
        findings,
        observed_at: Utc::now().to_rfc3339(),
    }))
}

/// The internal database resources this application can join, with the current
/// state of each link. Reachability is what makes a resource listable here, so
/// an empty list means the environment has no database resource on this server.
pub async fn resource_networks_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppRead>,
    Path(_): Path<ApplicationPath>,
) -> Result<impl IntoResponse, AppError> {
    Ok(ok(
        resource_networks::overview(&state, &app.application_id).await?
    ))
}

pub async fn attach_resource_network_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, resource_id)): Path<ResourcePath>,
) -> Result<impl IntoResponse, AppError> {
    resource_networks::attach(&state, &app.application_id, &resource_id).await?;
    let _ = crate::git::repositories::audit_log::record(
        &state,
        &app.organization_id,
        &app.user,
        "network.attached",
        Some("resource"),
        Some(&resource_id),
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    Ok(ok(
        resource_networks::overview(&state, &app.application_id).await?
    ))
}

pub async fn detach_resource_network_handler(
    State(state): State<SharedState>,
    RequireApp(app, _): RequireApp<perm::AppUpdate>,
    // The extractor covers the leading segments.
    Path((_, _, _, _, resource_id)): Path<ResourcePath>,
) -> Result<impl IntoResponse, AppError> {
    resource_networks::detach(&state, &app.application_id, &resource_id).await?;
    let _ = crate::git::repositories::audit_log::record(
        &state,
        &app.organization_id,
        &app.user,
        "network.detached",
        Some("resource"),
        Some(&resource_id),
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    Ok(ok(
        resource_networks::overview(&state, &app.application_id).await?
    ))
}
