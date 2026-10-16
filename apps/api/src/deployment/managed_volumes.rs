use std::ffi::OsString;
use std::path::Path;

use chrono::Utc;
use podman_api::{Podman, models, opts::VolumeCreateOpts};
use uuid::Uuid;

use crate::{
    dtos::{
        deployment::{CreateManagedVolumeRequest, ManagedVolumeDto},
        response::AppError,
    },
    git::repositories::db,
    podman::containers::volume_workspace::WorkspaceRoot,
    servers::runtime::ServerRuntime,
    state::state::SharedState,
};

#[derive(Debug, Clone)]
struct ManagedVolumeRecord {
    dto: ManagedVolumeDto,
    podman_name: String,
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn normalize_name(value: &str) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 100 {
        return Err(AppError::ValidationError(
            "Volume name must contain 1 to 100 characters".into(),
        ));
    }
    Ok(value.to_string())
}

fn normalize_mount_path(value: &str) -> Result<String, AppError> {
    let value = value.trim();
    let path = Path::new(value);
    if value.is_empty() || value == "/" || !path.is_absolute() || value.contains('\0') {
        return Err(AppError::ValidationError(
            "Mount path must be an absolute path other than /".into(),
        ));
    }
    Ok(value.trim_end_matches('/').to_string())
}

fn normalize_service(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

fn validate_create_input(
    is_compose: bool,
    request: CreateManagedVolumeRequest,
    compose_services: &[String],
) -> Result<(String, String, Option<String>), AppError> {
    let name = normalize_name(&request.name)?;
    let mount_path = normalize_mount_path(&request.mount_path)?;
    let compose_service = normalize_service(request.compose_service);
    if is_compose {
        let service = compose_service.ok_or_else(|| {
            AppError::ValidationError(
                "Choose the Compose service that will receive this volume".into(),
            )
        })?;
        if !compose_services
            .iter()
            .any(|candidate| candidate == &service)
        {
            return Err(AppError::ValidationError(format!(
                "Compose service '{service}' was not found"
            )));
        }
        Ok((name, mount_path, Some(service)))
    } else if compose_service.is_some() {
        Err(AppError::ValidationError(
            "A target service is only valid for Compose applications".into(),
        ))
    } else {
        Ok((name, mount_path, None))
    }
}

fn row_to_record(row: &sqlx::sqlite::SqliteRow) -> Result<ManagedVolumeRecord, AppError> {
    use sqlx::Row;
    Ok(ManagedVolumeRecord {
        dto: ManagedVolumeDto {
            id: row.try_get(0)?,
            name: row.try_get(1)?,
            mount_path: row.try_get(2)?,
            compose_service: row.try_get(3)?,
            attached: row.try_get::<i64, _>(4)? != 0,
            created_at: row.try_get(5)?,
            updated_at: row.try_get(6)?,
        },
        podman_name: row.try_get(7)?,
    })
}

async fn record(
    state: &SharedState,
    application_id: &str,
    volume_id: &str,
) -> Result<ManagedVolumeRecord, AppError> {
    let conn = db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT id, name, mount_path, compose_service, attached, created_at, updated_at, podman_name FROM application_managed_volumes WHERE application_id = ?1 AND id = ?2",
    )
    .bind(application_id)
    .bind(volume_id)
    .fetch_optional(&conn)
    .await?
    .ok_or(AppError::ResourceNotFound)?;
    row_to_record(&row)
}

async fn has_mount_conflict(
    state: &SharedState,
    application_id: &str,
    mount_path: &str,
    compose_service: Option<&str>,
    excluding_id: Option<&str>,
) -> Result<bool, AppError> {
    use sqlx::Row;
    let conn = db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT COUNT(*) FROM application_managed_volumes WHERE application_id = ?1 AND attached = 1 AND mount_path = ?2 AND COALESCE(compose_service, '') = COALESCE(?3, '') AND (?4 IS NULL OR id <> ?4)",
    )
    .bind(application_id)
    .bind(mount_path)
    .bind(compose_service)
    .bind(excluding_id)
    .fetch_optional(&conn)
    .await?
    .ok_or(AppError::ResourceNotFound)?;
    Ok(row.try_get::<i64, _>(0)? > 0)
}

async fn update_attachment(
    state: &SharedState,
    application_id: &str,
    volume_id: &str,
    attached: bool,
) -> Result<ManagedVolumeDto, AppError> {
    let existing = record(state, application_id, volume_id).await?;
    if existing.dto.attached == attached {
        return Ok(existing.dto);
    }
    if attached
        && has_mount_conflict(
            state,
            application_id,
            &existing.dto.mount_path,
            existing.dto.compose_service.as_deref(),
            Some(volume_id),
        )
        .await?
    {
        return Err(AppError::ValidationError(
            "Another attached volume already uses this mount path for the same container".into(),
        ));
    }
    let updated_at = now_ms();
    let conn = db::get_conn(state).await;
    sqlx::query(
        "UPDATE application_managed_volumes SET attached = ?1, updated_at = ?2 WHERE application_id = ?3 AND id = ?4",
    )
    .bind(i64::from(attached))
    .bind(updated_at)
    .bind(application_id)
    .bind(volume_id)
    .execute(&conn)
    .await?;
    Ok(ManagedVolumeDto {
        attached,
        updated_at,
        ..existing.dto
    })
}

pub async fn list(
    state: &SharedState,
    application_id: &str,
) -> Result<Vec<ManagedVolumeDto>, AppError> {
    let conn = db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT id, name, mount_path, compose_service, attached, created_at, updated_at, podman_name FROM application_managed_volumes WHERE application_id = ?1 ORDER BY created_at, id",
    )
    .bind(application_id)
    .fetch_all(&conn)
    .await?;
    rows.iter().map(|row| Ok(row_to_record(row)?.dto)).collect()
}

pub async fn create_record(
    state: &SharedState,
    application_id: &str,
    is_compose: bool,
    request: CreateManagedVolumeRequest,
    compose_services: &[String],
) -> Result<ManagedVolumeDto, AppError> {
    let (name, mount_path, compose_service) =
        validate_create_input(is_compose, request, compose_services)?;
    if has_mount_conflict(
        state,
        application_id,
        &mount_path,
        compose_service.as_deref(),
        None,
    )
    .await?
    {
        return Err(AppError::ValidationError(
            "Another attached volume already uses this mount path for the same container".into(),
        ));
    }
    let id = Uuid::now_v7().to_string();
    let podman_name = format!("litepod-managed-{id}");
    let runtime =
        crate::servers::runtime::ServerRuntime::for_application(state, application_id).await?;
    if runtime.is_remote() {
        runtime
            .podman(&[
                OsString::from("volume"),
                OsString::from("create"),
                OsString::from(&podman_name),
            ])
            .await?;
    } else {
        let podman = Podman::new(&state.podman_socket)
            .map_err(|error| AppError::InternalError(format!("Invalid Podman socket: {error}")))?;
        podman
            .volumes()
            .create(&VolumeCreateOpts::builder().name(&podman_name).build())
            .await
            .map_err(|error| {
                AppError::InternalError(format!("Could not create managed volume: {error}"))
            })?;
    }
    let timestamp = now_ms();
    let conn = db::get_conn(state).await;
    let insert = sqlx::query(
        "INSERT INTO application_managed_volumes (id, application_id, name, podman_name, mount_path, compose_service, attached, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?7)",
    )
    .bind(id.clone())
    .bind(application_id)
    .bind(name.clone())
    .bind(podman_name.clone())
    .bind(mount_path.clone())
    .bind(compose_service.clone())
    .bind(timestamp)
    .execute(&conn)
    .await;
    if let Err(error) = insert {
        if !runtime.is_remote() {
            let podman = Podman::new(&state.podman_socket).map_err(|error| {
                AppError::InternalError(format!("Invalid Podman socket: {error}"))
            })?;
            let _ = podman.volumes().get(&podman_name).remove().await;
        } else {
            let _ = runtime
                .podman(&[
                    OsString::from("volume"),
                    OsString::from("rm"),
                    OsString::from(&podman_name),
                ])
                .await;
        }
        return Err(error.into());
    }
    Ok(ManagedVolumeDto {
        id,
        name,
        mount_path,
        compose_service,
        attached: true,
        created_at: timestamp,
        updated_at: timestamp,
    })
}

pub async fn attach(
    state: &SharedState,
    application_id: &str,
    volume_id: &str,
) -> Result<ManagedVolumeDto, AppError> {
    update_attachment(state, application_id, volume_id, true).await
}

pub async fn detach(
    state: &SharedState,
    application_id: &str,
    volume_id: &str,
) -> Result<ManagedVolumeDto, AppError> {
    update_attachment(state, application_id, volume_id, false).await
}

pub async fn delete(
    state: &SharedState,
    application_id: &str,
    volume_id: &str,
) -> Result<(), AppError> {
    let existing = record(state, application_id, volume_id).await?;
    if existing.dto.attached {
        return Err(AppError::ValidationError(
            "Detach this volume before deleting it".into(),
        ));
    }
    let runtime =
        crate::servers::runtime::ServerRuntime::for_application(state, application_id).await?;
    if runtime.is_remote() {
        runtime
            .podman(&[
                OsString::from("volume"),
                OsString::from("rm"),
                OsString::from(&existing.podman_name),
            ])
            .await?;
    } else {
        let podman = Podman::new(&state.podman_socket)
            .map_err(|error| AppError::InternalError(format!("Invalid Podman socket: {error}")))?;
        podman
            .volumes()
            .get(&existing.podman_name)
            .remove()
            .await
            .map_err(|error| {
                AppError::InternalError(format!("Could not delete managed volume: {error}"))
            })?;
    }
    state
        .volume_access
        .write()
        .await
        .remove(&existing.podman_name);
    let conn = db::get_conn(state).await;
    sqlx::query("DELETE FROM application_managed_volumes WHERE application_id = ?1 AND id = ?2")
        .bind(application_id)
        .bind(volume_id)
        .execute(&conn)
        .await?;
    Ok(())
}

pub async fn workspace_root(
    state: &SharedState,
    application_id: &str,
    volume_id: &str,
) -> Result<WorkspaceRoot, AppError> {
    let volume = record(state, application_id, volume_id).await?.podman_name;
    if ServerRuntime::for_application(state, application_id)
        .await?
        .is_remote()
    {
        Ok(WorkspaceRoot::RemoteNamedVolume {
            application_id: application_id.to_string(),
            volume,
        })
    } else {
        Ok(WorkspaceRoot::NamedVolume(volume))
    }
}

/// User-attached managed volumes, as libpod named volumes. These must go
/// through the container's dedicated `volumes` field (`NamedVolume`), not the
/// generic OCI `mounts` field with `type: "volume"` — the generic path skips
/// libpod's named-volume resolution (copy-up/chown), which crun then rejects
/// with "No such device" on rootless Podman.
pub async fn runtime_mounts(
    state: &SharedState,
    application_id: &str,
) -> Result<Vec<models::NamedVolume>, AppError> {
    use sqlx::Row;
    let conn = db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT podman_name, mount_path FROM application_managed_volumes WHERE application_id = ?1 AND attached = 1 ORDER BY created_at, id",
    )
    .bind(application_id)
    .fetch_all(&conn)
    .await?;
    let mut mounts = Vec::new();
    for row in &rows {
        mounts.push(models::NamedVolume {
            dest: Some(row.try_get(1)?),
            is_anonymous: None,
            name: Some(row.try_get(0)?),
            options: None,
        });
    }
    Ok(mounts)
}

pub fn append_runtime_mounts(
    mut existing: Vec<models::NamedVolume>,
    managed: Vec<models::NamedVolume>,
) -> Vec<models::NamedVolume> {
    existing.extend(managed);
    existing
}

pub async fn compose_override(
    state: &SharedState,
    application_id: &str,
) -> Result<String, AppError> {
    use sqlx::Row;
    let conn = db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT podman_name, mount_path, compose_service FROM application_managed_volumes WHERE application_id = ?1 AND attached = 1 AND compose_service IS NOT NULL ORDER BY created_at, id",
    )
    .bind(application_id)
    .fetch_all(&conn)
    .await?;
    let mut root = serde_yaml::Mapping::new();
    let mut services = serde_yaml::Mapping::new();
    let mut volumes = serde_yaml::Mapping::new();
    for row in &rows {
        let podman_name: String = row.try_get(0)?;
        let mount_path: String = row.try_get(1)?;
        let service: String = row.try_get(2)?;
        let service_key = serde_yaml::Value::String(service);
        let service_entry = services
            .entry(service_key)
            .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
        let service_map = service_entry
            .as_mapping_mut()
            .expect("managed volume service is a mapping");
        let mounts = service_map
            .entry(serde_yaml::Value::String("volumes".into()))
            .or_insert_with(|| serde_yaml::Value::Sequence(Vec::new()));
        mounts
            .as_sequence_mut()
            .expect("managed volume mounts are a sequence")
            .push(serde_yaml::Value::Mapping(serde_yaml::Mapping::from_iter(
                [
                    (
                        serde_yaml::Value::String("type".into()),
                        serde_yaml::Value::String("volume".into()),
                    ),
                    (
                        serde_yaml::Value::String("source".into()),
                        serde_yaml::Value::String(podman_name.clone()),
                    ),
                    (
                        serde_yaml::Value::String("target".into()),
                        serde_yaml::Value::String(mount_path),
                    ),
                ],
            )));
        volumes.insert(
            serde_yaml::Value::String(podman_name.clone()),
            serde_yaml::Value::Mapping(serde_yaml::Mapping::from_iter([
                (
                    serde_yaml::Value::String("external".into()),
                    serde_yaml::Value::Bool(true),
                ),
                (
                    serde_yaml::Value::String("name".into()),
                    serde_yaml::Value::String(podman_name),
                ),
            ])),
        );
    }
    if services.is_empty() {
        return Ok(String::new());
    }
    root.insert(
        serde_yaml::Value::String("services".into()),
        serde_yaml::Value::Mapping(services),
    );
    root.insert(
        serde_yaml::Value::String("volumes".into()),
        serde_yaml::Value::Mapping(volumes),
    );
    serde_yaml::to_string(&serde_yaml::Value::Mapping(root))
        .map_err(|error| AppError::InternalError(error.to_string()))
}

pub async fn attached_compose_services(
    state: &SharedState,
    application_id: &str,
) -> Result<Vec<String>, AppError> {
    use sqlx::Row;
    let conn = db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT DISTINCT compose_service FROM application_managed_volumes WHERE application_id = ?1 AND attached = 1 AND compose_service IS NOT NULL ORDER BY compose_service",
    )
    .bind(application_id)
    .fetch_all(&conn)
    .await?;
    let mut services = Vec::new();
    for row in &rows {
        services.push(row.try_get(0)?);
    }
    Ok(services)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_create_input_for_static_and_compose_apps() {
        let static_input = CreateManagedVolumeRequest {
            name: " uploads ".into(),
            mount_path: " /app/data/ ".into(),
            compose_service: None,
        };
        assert_eq!(
            validate_create_input(false, static_input, &[]).unwrap(),
            ("uploads".into(), "/app/data".into(), None)
        );
        for path in ["", "/", "data"] {
            let request = CreateManagedVolumeRequest {
                name: "uploads".into(),
                mount_path: path.into(),
                compose_service: None,
            };
            assert!(validate_create_input(false, request, &[]).is_err());
        }
        let services = vec!["web".to_string()];
        let compose_input = CreateManagedVolumeRequest {
            name: "uploads".into(),
            mount_path: "/app/data".into(),
            compose_service: Some("web".into()),
        };
        assert_eq!(
            validate_create_input(true, compose_input, &services)
                .unwrap()
                .2
                .as_deref(),
            Some("web")
        );
        let missing_service = CreateManagedVolumeRequest {
            name: "uploads".into(),
            mount_path: "/app/data".into(),
            compose_service: None,
        };
        assert!(validate_create_input(true, missing_service, &services).is_err());
    }

    #[test]
    fn appends_runtime_mounts_without_replacing_existing_ones() {
        let base = models::NamedVolume {
            dest: Some("/srv".into()),
            is_anonymous: None,
            name: Some("base".into()),
            options: None,
        };
        let managed = models::NamedVolume {
            dest: Some("/app/data".into()),
            is_anonymous: None,
            name: Some("managed".into()),
            options: None,
        };
        let mounts = append_runtime_mounts(vec![base], vec![managed]);
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[1].dest.as_deref(), Some("/app/data"));
    }
}
