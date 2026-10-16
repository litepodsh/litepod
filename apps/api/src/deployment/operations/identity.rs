use super::super::domains;

use std::path::PathBuf;

use sqlx::Row;

use crate::{
    dtos::{
        deployment::{ApplicationReplicaDto, ArtifactDto, DeploymentDto},
        response::AppError,
    },
    state::state::SharedState,
};

use super::resources::*;

pub(crate) fn deployment_identity(
    // Kept in the signature so the ~dozen call sites stay untouched; only the
    // application name still feeds the runtime identity.
    _project_name: &str,
    _env_name: &str,
    app_name: &str,
    application_id: &str,
    public_ip: &str,
) -> (String, String) {
    let slug = |s: &str| {
        s.to_lowercase()
            .replace(|c: char| !c.is_alphanumeric(), "-")
    };
    // Only the app slug plus the stable id suffix. Renaming the project or the
    // environment now leaves every container name and Caddy dial target
    // unchanged. Renaming the application itself still recreates on the next
    // deploy; the forced pre-deploy sweep in `compose_standalone` clears the
    // old-named containers so that recreate succeeds.
    let name = format!(
        "{}-{}",
        slug(app_name),
        short_application_suffix(application_id)
    );
    (name, format!("{application_id}.{public_ip}.sslip.io"))
}

pub(super) fn app_root(state: &SharedState, application_id: &str) -> PathBuf {
    state.data_dir.join("applications").join(application_id)
}

/// The canonical `(container_name, hostname)` pair for an application, derived
/// from its project/environment/application names. Deploy pipelines must call
/// this (not invent their own scheme) so the podman container name, the
/// `application_deployments.container_name` column, and the Caddy dial target
/// always agree.
pub(crate) async fn identity(
    state: &SharedState,
    application_id: &str,
) -> Result<(String, String), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT p.name, e.name, a.name FROM applications a INNER JOIN environments e ON e.id = a.environment_id INNER JOIN projects p ON p.id = e.project_id WHERE a.id = ?1",
    )
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ApplicationNotFound)?;
    let project_name: String = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let env_name: String = row
        .try_get(1)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let app_name: String = row
        .try_get(2)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(deployment_identity(
        &project_name,
        &env_name,
        &app_name,
        application_id,
        &state.public_ip,
    ))
}

pub async fn ensure_deployment(state: &SharedState, application_id: &str) -> Result<(), AppError> {
    let (container_name, hostname) = identity(state, application_id).await?;
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT OR IGNORE INTO application_deployments (application_id, status, container_name, hostname, updated_at) VALUES (?1, 'not_deployed', ?2, ?3, ?4)",
    )
    .bind(application_id)
    .bind(container_name)
    .bind(hostname)
    .bind(now_ms())
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

pub async fn deployment(
    state: &SharedState,
    application_id: &str,
) -> Result<DeploymentDto, AppError> {
    ensure_deployment(state, application_id).await?;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT d.status, d.config_stale, d.last_error, d.deployed_at, a.id, a.filename, a.kind, a.byte_size, a.sha256, a.created_at FROM application_deployments d LEFT JOIN application_artifacts a ON a.id = d.artifact_id WHERE d.application_id = ?1",
    )
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    .ok_or(AppError::ApplicationNotFound)?;
    let artifact_id: Option<String> = row
        .try_get(4)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let artifact = artifact_id
        .map(|id| {
            Ok::<_, AppError>(ArtifactDto {
                id,
                filename: row
                    .try_get(5)
                    .map_err(|e| AppError::DatabaseError(e.to_string()))?,
                kind: row
                    .try_get(6)
                    .map_err(|e| AppError::DatabaseError(e.to_string()))?,
                byte_size: row
                    .try_get(7)
                    .map_err(|e| AppError::DatabaseError(e.to_string()))?,
                sha256: row
                    .try_get(8)
                    .map_err(|e| AppError::DatabaseError(e.to_string()))?,
                created_at: row
                    .try_get(9)
                    .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            })
        })
        .transpose()?;
    let mut status: String = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let config_stale = row
        .try_get::<i64, _>(1)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        != 0;
    let last_error: Option<String> = row
        .try_get(2)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let deployed_at: Option<i64> = row
        .try_get(3)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    drop(conn);
    let application_domains = domains::list(state, application_id).await?;
    let public_url = application_domains
        .iter()
        .find(|domain| domain.is_primary)
        .map(|domain| domain.url.clone());
    let container_port = app_container_port(state, application_id).await?;
    let mut replicas = Vec::new();
    for record in crate::deployment::replicas::list(state, application_id).await? {
        let server = crate::servers::repository::get(state, &record.server_id).await?;
        let (os, distribution, address) = server
            .as_ref()
            .map(crate::dtos::server::replica_marks)
            .unwrap_or_default();
        replicas.push(ApplicationReplicaDto {
            server_id: record.server_id,
            server_name: server.map(|server| server.name).unwrap_or_default(),
            os,
            distribution,
            address,
            status: record.status,
            last_error: record.last_error,
            deployed_at: record.deployed_at,
        });
    }
    // A group app's per-replica states are authoritative: the single
    // `application_deployments.status` only reflects whichever replica finished
    // last, so derive the aggregate status from the replicas instead.
    if !replicas.is_empty()
        && let Some(aggregate) =
            crate::deployment::replicas::aggregate(state, application_id).await?
    {
        status = aggregate;
    }
    Ok(DeploymentDto {
        status,
        container_port,
        domains: application_domains,
        public_url,
        config_stale,
        last_error,
        deployed_at,
        artifact,
        replicas,
    })
}
