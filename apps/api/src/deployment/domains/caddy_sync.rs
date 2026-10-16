use podman_api::models;
use sqlx::Row;

use crate::{
    dtos::{deployment::*, response::AppError},
    state::state::SharedState,
};

use super::crud::*;

pub fn route_id(domain_id: &str) -> String {
    format!("litepod-route-{domain_id}")
}

/// Published host ports for every local domain. Non-local domains reach the
/// container through Caddy on the shared network and need no mapping.
pub async fn port_mappings(
    state: &SharedState,
    application_id: &str,
) -> Result<Vec<models::PortMapping>, AppError> {
    let container_port = super::super::app_container_port(state, application_id).await?;
    Ok(list(state, application_id)
        .await?
        .into_iter()
        .filter(|domain| domain.is_local)
        .map(|domain| models::PortMapping {
            container_port: Some(container_port as u16),
            host_ip: Some("0.0.0.0".into()),
            host_port: Some(domain.port as u16),
            protocol: Some("tcp".into()),
            range: Some(1),
        })
        .collect())
}

pub async fn sync_caddy_route(
    state: &SharedState,
    domain: &DomainDto,
    container_name: &str,
) -> Result<(), AppError> {
    if !application_uses_local_caddy(state, &domain.application_id).await? {
        // Remote ingress is reconciled by `servers::ingress`; never point the
        // central Caddy at a container name that only exists on a slave.
        return Ok(());
    }
    push_caddy_route(state, domain, container_name, domain.port).await
}

/// Publishes the route for one domain. The dial target is passed in because it
/// differs per deploy type: a single container plus the domain's stored port for
/// static and git deploys, one service of the stack plus its own container port
/// for Compose.
pub(crate) async fn push_caddy_route(
    state: &SharedState,
    domain: &DomainDto,
    container_name: &str,
    container_port: i64,
) -> Result<(), AppError> {
    if domain.is_local {
        return Ok(());
    }
    super::super::caddy_route(
        state,
        &route_id(&domain.id),
        &domain.host,
        container_name,
        container_port,
        domain.https,
    )
    .await
}

pub async fn remove_caddy_route(
    state: &SharedState,
    application_id: &str,
    domain_id: &str,
    hostname: &str,
) {
    // Best-effort so a transient Caddy/edge outage can't block the row delete
    // that follows in `cleanup`, but a failure here means an orphaned fragment
    // that will later collide with any domain reusing the host, so it must not
    // vanish silently.
    let outcome = if application_uses_local_caddy(state, application_id)
        .await
        .unwrap_or(true)
    {
        super::super::remove_caddy_route_config(state, &route_id(domain_id), hostname).await
    } else {
        crate::servers::ingress::remove_domain(state, application_id, domain_id, hostname).await
    };
    if let Err(error) = outcome {
        tracing::warn!(
            application_id,
            domain_id,
            host = %hostname,
            %error,
            "could not remove Caddy route; fragment may be orphaned on the edge server"
        );
    }
}

/// Every route the database says Caddy should be serving right now, with the
/// upstream each one has to dial.
///
/// `dial` is `None` when the domain is not routable yet - the deployment is not
/// running, or a Compose domain has no service selected - which is exactly the
/// state where a live Caddy route would 502.
pub async fn expected_routes(state: &SharedState) -> Result<Vec<ExpectedRouteDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT DISTINCT d.application_id, a.name FROM application_domains d \
             LEFT JOIN applications a ON a.id = d.application_id",
    )
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut applications = Vec::new();
    for row in &rows {
        let id: String = row
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let name: Option<String> = row
            .try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        applications.push((id, name.unwrap_or_default()));
    }
    drop(conn);

    let mut expected = Vec::new();
    for (application_id, application_name) in applications {
        if !application_uses_local_caddy(state, &application_id).await? {
            continue;
        }
        for domain in list(state, &application_id)
            .await?
            .into_iter()
            .filter(|domain| !domain.is_local)
        {
            let target = caddy_target(state, &application_id, &domain).await?;
            expected.push(ExpectedRouteDto {
                route_id: route_id(&domain.id),
                application_id: application_id.clone(),
                application_name: application_name.clone(),
                domain_id: domain.id,
                host: domain.host,
                https: domain.https,
                dial: target.map(|(name, port)| format!("{name}:{port}")),
            });
        }
    }
    Ok(expected)
}

/// One non-local application domain that resolves to a server, paired with its
/// edge fragment id (the domain id) and whether it currently has a loopback
/// endpoint the edge Caddy could dial. Feeds the Caddy console's disk-vs-DB
/// diff and the declarative route manifest.
#[derive(Debug, Clone)]
pub struct EdgeRouteTarget {
    pub domain_id: String,
    pub host: String,
    pub application_name: String,
    pub routable: bool,
}

/// Every non-local domain on one server, with its id and routable state.
pub async fn edge_route_targets(
    state: &SharedState,
    server_id: &str,
) -> Result<Vec<EdgeRouteTarget>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query("SELECT id, name FROM applications WHERE server_id = ?1")
        .bind(server_id)
        .fetch_all(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut applications = Vec::new();
    for row in &rows {
        let id: String = row
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let name: Option<String> = row
            .try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        applications.push((id, name.unwrap_or_default()));
    }
    drop(conn);

    let mut targets = Vec::new();
    for (application_id, application_name) in applications {
        for domain in list(state, &application_id)
            .await?
            .into_iter()
            .filter(|domain| !domain.is_local)
        {
            let routable = crate::servers::ingress::domain_endpoint(state, &domain)
                .await?
                .is_some();
            targets.push(EdgeRouteTarget {
                domain_id: domain.id,
                host: domain.host,
                application_name: application_name.clone(),
                routable,
            });
        }
    }
    Ok(targets)
}

/// Re-pushes every routable domain into Caddy.
///
/// Caddy holds its configuration in memory only, so a Caddy restart - the core
/// updater recreates that container on every version bump - drops every
/// application route. Nothing else puts them back: `restore_caddy` only reinjects
/// the server's own domain, so without this the apps stay dark until each one is
/// redeployed by hand.
///
/// Returns `(pushed, failed)`. One broken route must not stop the rest, so
/// failures are logged and counted instead of returned.
pub async fn reconcile_all_routes(state: &SharedState) -> Result<(usize, usize), AppError> {
    let mut pushed = 0;
    let mut failed = 0;
    for route in expected_routes(state).await? {
        let Some(dial) = route.dial.as_deref() else {
            continue;
        };
        let Some((container_name, container_port)) = dial.rsplit_once(':') else {
            continue;
        };
        let Ok(container_port) = container_port.parse::<i64>() else {
            continue;
        };
        match super::super::caddy_route(
            state,
            &route.route_id,
            &route.host,
            container_name,
            container_port,
            route.https,
        )
        .await
        {
            Ok(()) => pushed += 1,
            Err(error) => {
                failed += 1;
                tracing::warn!(host = %route.host, %error, "could not reconcile Caddy route");
            }
        }
    }
    Ok((pushed, failed))
}

/// Flags that published ports in the database no longer match the running
/// container, so the UI can prompt for a redeploy.
pub async fn mark_stale(
    state: &SharedState,
    application_id: &str,
    stale: bool,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE application_deployments SET config_stale = ?1, updated_at = ?2 WHERE application_id = ?3")
        .bind(stale as i64)
        .bind(now_ms())
        .bind(application_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

/// Where Caddy has to dial for one domain, as `(container_name, container_port)`.
///
/// `None` means the domain is not routable yet, either because the deployment is
/// not running or because a Compose domain has no service selected.
pub(crate) async fn caddy_target(
    state: &SharedState,
    application_id: &str,
    domain: &DomainDto,
) -> Result<Option<(String, i64)>, AppError> {
    let Some(name) = container_name(state, application_id).await? else {
        return Ok(None);
    };
    if !is_compose_stack(state, application_id).await? {
        return Ok(Some((name, domain.port)));
    }
    // A Compose deployment stores the *project* name, which is not a container.
    // The domain's proxy route names the one service Caddy should reach.
    let Some(route) = super::super::compose::proxy_routes::list(state, application_id)
        .await?
        .into_iter()
        .find(|route| route.domain_id == domain.id)
    else {
        return Ok(None);
    };
    let runtime_name = super::super::rollout::active_runtime_name(state, application_id)
        .await
        .map_err(AppError::InternalError)?;
    Ok(Some((
        format!("{runtime_name}-{}", route.service),
        route.container_port as i64,
    )))
}

/// Whether the application deploys as a Compose stack (catalog or pasted).
pub async fn is_compose_application(
    state: &SharedState,
    application_id: &str,
) -> Result<bool, AppError> {
    is_compose_stack(state, application_id).await
}

pub(super) async fn is_compose_stack(
    state: &SharedState,
    application_id: &str,
) -> Result<bool, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let Some(row) = sqlx::query("SELECT deploy_type FROM applications WHERE id = ?1")
        .bind(application_id)
        .fetch_optional(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
    else {
        return Ok(false);
    };
    let deploy_type: String = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(deploy_type == "compose")
}

/// litepod's embedded Caddy may only ever resolve Local Podman names. SSH
/// servers use a native edge Caddy or a relay target and are intentionally
/// excluded here until `servers::ingress` reconciles their route. A server-group
/// application has no `server_id` and the local server can never join a group,
/// so it never uses the embedded Caddy either.
pub(crate) async fn application_uses_local_caddy(
    state: &SharedState,
    application_id: &str,
) -> Result<bool, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT s.kind FROM applications a LEFT JOIN servers s ON s.id = a.server_id WHERE a.id = ?1",
    )
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|error| AppError::DatabaseError(error.to_string()))?
    .ok_or(AppError::ApplicationNotFound)?;
    let kind: Option<String> = row
        .try_get(0)
        .map_err(|error| AppError::DatabaseError(error.to_string()))?;
    Ok(kind.as_deref() == Some("local"))
}

/// Container name for an application's static deployment, needed to dial it
/// from a Caddy route. `None` unless the container is running, because a route
/// pointing at a stopped container would 502.
pub(super) async fn container_name(
    state: &SharedState,
    application_id: &str,
) -> Result<Option<String>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let Some(row) = sqlx::query(
        "SELECT container_name, status FROM application_deployments WHERE application_id = ?1",
    )
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    else {
        return Ok(None);
    };
    let name: String = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let status: String = row
        .try_get(1)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(if status == "running" {
        Some(name)
    } else {
        None
    })
}

/// Caddy routes apply immediately because pushing one is cheap and causes no
/// downtime. Published ports need the container recreated, so those are flagged
/// for the next deploy instead.
pub(super) async fn reconcile_after_write(
    state: &SharedState,
    application_id: &str,
    domain: &DomainDto,
    previous: Option<&DomainDto>,
) -> Result<(), AppError> {
    let port_changed = previous
        .is_none_or(|before| before.is_local != domain.is_local || before.port != domain.port);
    if (domain.is_local || previous.is_some_and(|before| before.is_local)) && port_changed {
        mark_stale(state, application_id, true).await?;
    }
    // A host that stopped being proxied leaves a stale route behind.
    if let Some(before) = previous
        && !before.is_local
        && (before.host != domain.host || domain.is_local)
    {
        remove_caddy_route(state, application_id, &before.id, &before.host).await;
    }
    if application_uses_local_caddy(state, application_id).await?
        && !domain.is_local
        && let Some((name, container_port)) = caddy_target(state, application_id, domain).await?
        && let Err(error) = push_caddy_route(state, domain, &name, container_port).await
    {
        return Err(error);
    }
    if !application_uses_local_caddy(state, application_id).await? && !domain.is_local {
        crate::servers::ingress::sync_domain(state, domain).await?;
    }
    Ok(())
}
