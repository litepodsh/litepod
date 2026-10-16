use crate::{
    deployment::domains,
    dtos::{deployment::ComposeProxyRouteDto, response::AppError},
    state::state::SharedState,
};
use chrono::Utc;
use sqlx::Row;

fn row(row: &sqlx::sqlite::SqliteRow) -> Result<ComposeProxyRouteDto, AppError> {
    Ok(ComposeProxyRouteDto {
        domain_id: row
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        application_id: row
            .try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        service: row
            .try_get(2)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        container_port: row
            .try_get::<i64, _>(3)
            .map_err(|e| AppError::DatabaseError(e.to_string()))? as u16,
    })
}
pub async fn list(
    state: &SharedState,
    application_id: &str,
) -> Result<Vec<ComposeProxyRouteDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query("SELECT domain_id, application_id, service, container_port FROM application_compose_proxy_routes WHERE application_id = ?1 ORDER BY domain_id").bind(application_id).fetch_all(&conn).await.map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut out = Vec::new();
    for item in &rows {
        out.push(row(item)?);
    }
    Ok(out)
}
pub async fn upsert(
    state: &SharedState,
    application_id: &str,
    domain_id: &str,
    service: &str,
    container_port: u16,
) -> Result<ComposeProxyRouteDto, AppError> {
    if container_port == 0 {
        return Err(AppError::ValidationError(
            "Container port must be between 1 and 65535".into(),
        ));
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    let domain_exists = {
        let row =
            sqlx::query("SELECT 1 FROM application_domains WHERE id = ?1 AND application_id = ?2")
                .bind(domain_id)
                .bind(application_id)
                .fetch_optional(&conn)
                .await
                .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        row.is_some()
    };
    if !domain_exists {
        return Err(AppError::ResourceNotFound);
    }
    let now = Utc::now().timestamp_millis();
    crate::git::repositories::db::retry_on_stale_snapshot(|| async {
        sqlx::query("INSERT INTO application_compose_proxy_routes (domain_id, application_id, service, container_port, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5) ON CONFLICT(domain_id) DO UPDATE SET application_id = excluded.application_id, service = excluded.service, container_port = excluded.container_port, updated_at = excluded.updated_at")
            .bind(domain_id)
            .bind(application_id)
            .bind(service)
            .bind(container_port as i64)
            .bind(now)
            .execute(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    })
    .await?;
    let fetched = sqlx::query("SELECT domain_id, application_id, service, container_port FROM application_compose_proxy_routes WHERE domain_id = ?1").bind(domain_id).fetch_optional(&conn).await.map_err(|e| AppError::DatabaseError(e.to_string()))?;
    row(&fetched.ok_or(AppError::ResourceNotFound)?)
}
/// Stores a route and applies what can be applied without downtime.
///
/// The Caddy route is pushed immediately, like every other domain write. Joining
/// the shared proxy network cannot be done without recreating the container, so
/// a service that was not routed before only marks the deployment stale and the
/// UI asks for a redeploy.
pub async fn save(
    state: &SharedState,
    application_id: &str,
    domain_id: &str,
    service: &str,
    container_port: u16,
) -> Result<ComposeProxyRouteDto, AppError> {
    let already_routed = list(state, application_id)
        .await?
        .iter()
        .any(|route| route.service == service);
    let saved = upsert(state, application_id, domain_id, service, container_port).await?;

    let domain = domains::get(state, application_id, domain_id).await?;
    if let Some((container_name, container_port)) =
        domains::caddy_target(state, application_id, &domain).await?
    {
        domains::push_caddy_route(state, &domain, &container_name, container_port).await?;
    }
    if !already_routed
        || crate::deployment::compose::marker_routes::references_route(
            state,
            application_id,
            service,
            container_port,
        )
        .await?
    {
        domains::mark_stale(state, application_id, true).await?;
    }
    Ok(saved)
}

/// Drops a route and takes its Caddy route down with it, so the domain stops
/// answering instead of silently falling through to litepod's catch-all.
pub async fn remove(
    state: &SharedState,
    application_id: &str,
    domain_id: &str,
) -> Result<(), AppError> {
    let removed = list(state, application_id)
        .await?
        .into_iter()
        .find(|route| route.domain_id == domain_id);
    delete_by_domain(state, application_id, domain_id).await?;
    if removed.is_none() {
        return Ok(());
    }
    if let Ok(domain) = domains::get(state, application_id, domain_id).await {
        domains::remove_caddy_route(state, application_id, domain_id, &domain.host).await;
    }
    domains::mark_stale(state, application_id, true).await?;
    Ok(())
}

pub async fn delete_by_domain(
    state: &SharedState,
    application_id: &str,
    domain_id: &str,
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "DELETE FROM application_compose_proxy_routes WHERE application_id = ?1 AND domain_id = ?2",
    )
    .bind(application_id)
    .bind(domain_id)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}
