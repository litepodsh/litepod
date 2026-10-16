//! Named S3 destinations that database backup policies can target.
//!
//! Each target belongs to one organization (`organization_id`) and stores its
//! credentials encrypted at rest (same envelope scheme as the global
//! `s3_config`). Every read and write here is scoped by organization: a target
//! of another organization is indistinguishable from one that does not exist.
//! The only target without an organization is the default one, the operator's
//! system-backup destination, which is never offered to an organization.
//!
//! The database-backup pipeline resolves a policy's `s3_target_id` and
//! uploads/restores against that exact destination, recording the target id on
//! every run row. Falling back to the global config when a policy names no
//! target is self-hosted only: on cloud that config is the operator's bucket.

use chrono::Utc;
use sqlx::Row;
use zeroize::Zeroize;

use crate::backup::config::{Encrypted, S3Config};
use crate::dtos::response::AppError;
use crate::state::state::SharedState;

/// Full target with the secret decrypted, for internal use only.
#[derive(Clone)]
pub struct S3Target {
    pub id: String,
    pub name: String,
    pub endpoint_url: String,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
}

/// Client-facing shape: the secret is never returned.
#[derive(Debug, serde::Serialize)]
pub struct S3TargetDto {
    pub id: String,
    pub name: String,
    pub endpoint_url: String,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    pub has_secret_key: bool,
    pub is_default: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, serde::Deserialize)]
pub struct CreateS3TargetRequest {
    pub name: String,
    pub endpoint_url: String,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct UpdateS3TargetRequest {
    pub name: String,
    pub endpoint_url: String,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    /// `None` keeps the existing encrypted secret.
    #[serde(default)]
    pub secret_access_key: Option<String>,
}

fn validate(req: &CreateS3TargetRequest, require_secret: bool) -> Result<(), AppError> {
    let missing = [
        (req.name.trim().is_empty(), "name"),
        (req.endpoint_url.trim().is_empty(), "endpoint"),
        (req.bucket.trim().is_empty(), "bucket"),
        (req.region.trim().is_empty(), "region"),
        (req.access_key_id.trim().is_empty(), "access key id"),
    ]
    .iter()
    .filter(|(empty, _)| *empty)
    .map(|(_, field)| *field)
    .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(AppError::ValidationError(format!(
            "Missing required fields: {}",
            missing.join(", ")
        )));
    }
    if require_secret && req.secret_access_key.trim().is_empty() {
        return Err(AppError::ValidationError(
            "A secret access key is required".into(),
        ));
    }
    Ok(())
}

fn read_target_row(row: &sqlx::sqlite::SqliteRow) -> Result<S3Target, AppError> {
    let err = |e: sqlx::Error| AppError::DatabaseError(e.to_string());
    let id: String = row.try_get(0).map_err(err)?;
    let name: String = row.try_get(1).map_err(err)?;
    let endpoint_url: String = row.try_get(2).map_err(err)?;
    let bucket: String = row.try_get(3).map_err(err)?;
    let region: String = row.try_get(4).map_err(err)?;
    let access_key_id: String = row.try_get(5).map_err(err)?;
    let wrapped_dek: Vec<u8> = row.try_get(6).map_err(err)?;
    let dek_nonce: Vec<u8> = row.try_get(7).map_err(err)?;
    let ciphertext: Vec<u8> = row.try_get(8).map_err(err)?;
    let nonce: Vec<u8> = row.try_get(9).map_err(err)?;
    let sealed = Encrypted {
        wrapped_dek,
        dek_nonce,
        ciphertext,
        nonce,
    };
    let mut secret_bytes =
        crate::backup::config::open_secret(&sealed).map_err(AppError::InternalError)?;
    let secret_access_key = String::from_utf8(secret_bytes.clone())
        .map_err(|_| AppError::InternalError("decrypted S3 secret is not utf-8".into()))?;
    secret_bytes.zeroize();
    Ok(S3Target {
        id,
        name,
        endpoint_url,
        bucket,
        region,
        access_key_id,
        secret_access_key,
    })
}

/// DTO from a row that includes `created_at`, `updated_at` at columns 10-11 and
/// `is_default` at column 12.
fn read_dto(row: &sqlx::sqlite::SqliteRow) -> Result<S3TargetDto, AppError> {
    let err = |e: sqlx::Error| AppError::DatabaseError(e.to_string());
    let target = read_target_row(row)?;
    let created_at: i64 = row.try_get(10).map_err(err)?;
    let updated_at: i64 = row.try_get(11).map_err(err)?;
    let is_default: i64 = row.try_get(12).map_err(err)?;
    Ok(S3TargetDto {
        id: target.id,
        name: target.name,
        endpoint_url: target.endpoint_url,
        bucket: target.bucket,
        region: target.region,
        access_key_id: target.access_key_id,
        has_secret_key: true,
        is_default: is_default != 0,
        created_at,
        updated_at,
    })
}

const TARGET_DTO_COLUMNS: &str = "id, name, endpoint_url, bucket, region, access_key_id, wrapped_dek, dek_nonce, ciphertext, nonce, created_at, updated_at, is_default";
const TARGET_ROW_COLUMNS: &str = "id, name, endpoint_url, bucket, region, access_key_id, wrapped_dek, dek_nonce, ciphertext, nonce";

/// One organization's targets, name-sorted so the UI selector is stable.
pub async fn list(
    state: &SharedState,
    organization_id: &str,
) -> Result<Vec<S3TargetDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {TARGET_DTO_COLUMNS} FROM backup_s3_targets \
         WHERE organization_id = ?1 AND is_default = 0 ORDER BY name ASC"
    ))
    .bind(organization_id)
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    rows.iter().map(|row| read_dto(row)).collect()
}

/// One organization's targets without decrypting secrets — plain id/name for
/// selectors.
pub async fn list_meta(
    state: &SharedState,
    organization_id: &str,
) -> Result<Vec<(String, String)>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT id, name FROM backup_s3_targets \
         WHERE organization_id = ?1 AND is_default = 0 ORDER BY name ASC",
    )
    .bind(organization_id)
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    rows.iter()
        .map(|row| {
            let id: String = row
                .try_get(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?;
            let name: String = row
                .try_get(1)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?;
            Ok((id, name))
        })
        .collect()
}

/// Loads one of the organization's targets with the secret decrypted.
/// `Ok(None)` when missing or owned by another organization.
pub async fn get(
    state: &SharedState,
    organization_id: &str,
    id: &str,
) -> Result<Option<S3Target>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(&format!(
        "SELECT {TARGET_ROW_COLUMNS} FROM backup_s3_targets \
         WHERE id = ?1 AND organization_id = ?2 AND is_default = 0"
    ))
    .bind(id)
    .bind(organization_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    match row {
        Some(row) => Ok(Some(read_target_row(&row)?)),
        None => Ok(None),
    }
}

/// Where a customer-supplied endpoint may point. Self-hosted keeps today's
/// freedom (a MinIO on the LAN is the normal case). On cloud the server dialing
/// it is the control plane, so the endpoint must be HTTPS and resolve to public
/// addresses only — checked again before every use, not just at save time,
/// because DNS can change after the target is saved.
pub async fn check_endpoint(is_cloud: bool, endpoint_url: &str) -> Result<(), AppError> {
    if !is_cloud {
        return Ok(());
    }
    let url = crate::security::ssrf::parse_https_url_with_port(endpoint_url)
        .map_err(|error| AppError::ValidationError(format!("S3 endpoint: {error}")))?;
    crate::security::ssrf::assert_public_host(&url)
        .await
        .map_err(|error| AppError::ValidationError(format!("S3 endpoint: {error}")))
}

/// A target's full connection details shaped like the global S3 config, so the
/// backup client builder (`backup::s3::client`) works unchanged.
pub fn to_config(target: &S3Target) -> S3Config {
    S3Config {
        endpoint_url: target.endpoint_url.clone(),
        bucket: target.bucket.clone(),
        region: target.region.clone(),
        access_key_id: target.access_key_id.clone(),
        secret_access_key: target.secret_access_key.clone(),
        schedule_enabled: false,
        schedule_interval: None,
        schedule_time: None,
    }
}

/// Resolves where a *new* backup of an application in `organization_id` goes:
/// the named target when the policy picks one (it must be this organization's),
/// the global config otherwise — self-hosted only.
pub async fn resolve(
    state: &SharedState,
    organization_id: &str,
    s3_target_id: Option<&str>,
) -> Result<S3Config, AppError> {
    if let Some(id) = s3_target_id {
        let target = get(state, organization_id, id).await?.ok_or_else(|| {
            AppError::ValidationError("The selected S3 target no longer exists".into())
        })?;
        check_endpoint(state.is_cloud, &target.endpoint_url).await?;
        return Ok(to_config(&target));
    }
    global_fallback(state).await
}

/// Resolves the destination an *existing* backup was uploaded to, for restore,
/// delete and retention. Besides the organization's own targets this accepts an
/// archived target (no organization: the original of a target that was shared
/// between organizations before targets were per organization), because older
/// backup rows still point at it. A backup row is only ever reached through its
/// own application, so this never widens what an organization can choose.
pub async fn resolve_recorded(
    state: &SharedState,
    organization_id: &str,
    s3_target_id: Option<&str>,
) -> Result<S3Config, AppError> {
    let Some(id) = s3_target_id else {
        return recorded_global(state).await;
    };
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(&format!(
        "SELECT {TARGET_ROW_COLUMNS} FROM backup_s3_targets \
         WHERE id = ?1 AND is_default = 0 AND (organization_id = ?2 OR organization_id IS NULL)"
    ))
    .bind(id)
    .bind(organization_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let Some(row) = row else {
        return Err(AppError::ValidationError(
            "The S3 target this backup was uploaded to no longer exists".into(),
        ));
    };
    let target = read_target_row(&row)?;
    check_endpoint(state.is_cloud, &target.endpoint_url).await?;
    Ok(to_config(&target))
}

/// The global destination for a policy that names no target. On cloud that is
/// the operator's own bucket, so a customer must bring a target instead.
async fn global_fallback(state: &SharedState) -> Result<S3Config, AppError> {
    if state.is_cloud {
        return Err(AppError::ValidationError(
            "Choose an S3 target for this database. Add one in your organization's \
             backup storage settings."
                .into(),
        ));
    }
    recorded_global(state).await
}

/// The global config, for archives that were uploaded there (self-hosted, or a
/// cloud backup taken before per-organization targets).
async fn recorded_global(state: &SharedState) -> Result<S3Config, AppError> {
    crate::backup::config::load(state).await?.ok_or_else(|| {
        AppError::ValidationError(
            "No S3 storage configured: set the global S3 destination or pick a backup target"
                .into(),
        )
    })
}

/// Creates a target. Callers should run `test_connection` against the built
/// config first — persist only on success (same sequencing as the global S3
/// config).
pub async fn create(
    state: &SharedState,
    organization_id: &str,
    req: &CreateS3TargetRequest,
    created_by: Option<&str>,
) -> Result<S3TargetDto, AppError> {
    validate(req, true)?;
    let now = Utc::now().timestamp_millis();
    let id = uuid::Uuid::now_v7().to_string();
    let mut secret_bytes = req.secret_access_key.as_bytes().to_vec();
    let sealed =
        crate::backup::config::seal_secret(&secret_bytes).map_err(AppError::InternalError)?;
    secret_bytes.zeroize();

    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO backup_s3_targets \
         (id, name, endpoint_url, bucket, region, access_key_id, wrapped_dek, dek_nonce, ciphertext, nonce, created_at, updated_at, created_by, organization_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
    )
    .bind(&id)
    .bind(req.name.trim())
    .bind(req.endpoint_url.trim())
    .bind(req.bucket.trim())
    .bind(req.region.trim())
    .bind(req.access_key_id.trim())
    .bind(sealed.wrapped_dek)
    .bind(sealed.dek_nonce)
    .bind(sealed.ciphertext)
    .bind(sealed.nonce)
    .bind(now)
    .bind(now)
    .bind(created_by)
    .bind(organization_id)
    .execute(&conn)
    .await
    .map_err(map_write_error)?;

    Ok(S3TargetDto {
        id,
        name: req.name.trim().to_string(),
        endpoint_url: req.endpoint_url.trim().to_string(),
        bucket: req.bucket.trim().to_string(),
        region: req.region.trim().to_string(),
        access_key_id: req.access_key_id.trim().to_string(),
        has_secret_key: true,
        is_default: false,
        created_at: now,
        updated_at: now,
    })
}

/// Updates a target. `secret_access_key: None` keeps the stored secret.
pub async fn update(
    state: &SharedState,
    organization_id: &str,
    id: &str,
    req: &UpdateS3TargetRequest,
) -> Result<S3TargetDto, AppError> {
    if req.name.trim().is_empty()
        || req.endpoint_url.trim().is_empty()
        || req.bucket.trim().is_empty()
        || req.region.trim().is_empty()
        || req.access_key_id.trim().is_empty()
    {
        return Err(AppError::ValidationError(
            "Name, endpoint, bucket, region, and access key id are required".into(),
        ));
    }
    let conn = crate::git::repositories::db::get_conn(state).await;

    let secret_to_store: (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) =
        match req.secret_access_key.as_deref().filter(|k| !k.is_empty()) {
            Some(provided) => {
                let mut secret_bytes = provided.as_bytes().to_vec();
                let sealed = crate::backup::config::seal_secret(&secret_bytes)
                    .map_err(AppError::InternalError)?;
                secret_bytes.zeroize();
                (
                    sealed.wrapped_dek,
                    sealed.dek_nonce,
                    sealed.ciphertext,
                    sealed.nonce,
                )
            }
            None => {
                let row = sqlx::query(
                    "SELECT wrapped_dek, dek_nonce, ciphertext, nonce FROM backup_s3_targets \
                 WHERE id = ?1 AND organization_id = ?2 AND is_default = 0",
                )
                .bind(id)
                .bind(organization_id)
                .fetch_optional(&conn)
                .await
                .map_err(|e| AppError::DatabaseError(e.to_string()))?;
                let Some(row) = row else {
                    return Err(AppError::ResourceNotFound);
                };
                let err = |e: sqlx::Error| AppError::DatabaseError(e.to_string());
                (
                    row.try_get(0).map_err(err)?,
                    row.try_get(1).map_err(err)?,
                    row.try_get(2).map_err(err)?,
                    row.try_get(3).map_err(err)?,
                )
            }
        };

    let now = Utc::now().timestamp_millis();
    let result = sqlx::query(
        "UPDATE backup_s3_targets SET name = ?1, endpoint_url = ?2, bucket = ?3, region = ?4, \
         access_key_id = ?5, wrapped_dek = ?6, dek_nonce = ?7, ciphertext = ?8, nonce = ?9, updated_at = ?10 \
         WHERE id = ?11 AND organization_id = ?12 AND is_default = 0",
    )
    .bind(req.name.trim())
    .bind(req.endpoint_url.trim())
    .bind(req.bucket.trim())
    .bind(req.region.trim())
    .bind(req.access_key_id.trim())
    .bind(secret_to_store.0)
    .bind(secret_to_store.1)
    .bind(secret_to_store.2)
    .bind(secret_to_store.3)
    .bind(now)
    .bind(id)
    .bind(organization_id)
    .execute(&conn)
    .await
    .map_err(map_write_error)?;
    if result.rows_affected() == 0 {
        return Err(AppError::ResourceNotFound);
    }

    Ok(S3TargetDto {
        id: id.to_string(),
        name: req.name.trim().to_string(),
        endpoint_url: req.endpoint_url.trim().to_string(),
        bucket: req.bucket.trim().to_string(),
        region: req.region.trim().to_string(),
        access_key_id: req.access_key_id.trim().to_string(),
        has_secret_key: true,
        is_default: false,
        created_at: now,
        updated_at: now,
    })
}

/// Deletes one of the organization's targets. Backups already uploaded to it
/// keep their records (their `s3_target_id` becomes a historical reference);
/// the organization's policies pointing at it are cleared, so on self-hosted
/// they fall back to the global config and on cloud they stop until a new
/// target is picked.
pub async fn delete(state: &SharedState, organization_id: &str, id: &str) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let result = sqlx::query(
        "DELETE FROM backup_s3_targets WHERE id = ?1 AND organization_id = ?2 AND is_default = 0",
    )
    .bind(id)
    .bind(organization_id)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    if result.rows_affected() == 0 {
        return Err(AppError::ResourceNotFound);
    }
    sqlx::query("UPDATE database_backup_policies SET s3_target_id = NULL WHERE s3_target_id = ?1")
        .bind(id)
        .execute(&conn)
        .await
        .ok();
    Ok(())
}

/// A duplicate name inside one organization reads as a validation error, not a
/// database failure.
fn map_write_error(error: sqlx::Error) -> AppError {
    if error
        .as_database_error()
        .is_some_and(|db| db.is_unique_violation())
    {
        return AppError::ValidationError(
            "This organization already has an S3 target with that name".into(),
        );
    }
    AppError::DatabaseError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dtos::database_backup::UpdateDatabaseBackupPolicyRequest;
    use crate::testing::{cleanup_database, seed_application, test_state_for};

    fn request(name: &str) -> CreateS3TargetRequest {
        CreateS3TargetRequest {
            name: name.into(),
            endpoint_url: "https://s3.example.com".into(),
            bucket: "bucket".into(),
            region: "us-east-1".into(),
            access_key_id: "AKIA".into(),
            secret_access_key: "secret".into(),
        }
    }

    fn policy(enabled: bool, target: Option<&str>) -> UpdateDatabaseBackupPolicyRequest {
        UpdateDatabaseBackupPolicyRequest {
            enabled,
            cron_expression: "0 0 3 * * *".into(),
            retention_count: 7,
            s3_target_id: target.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn another_organizations_target_is_invisible_and_untouchable() {
        let (state, path) = test_state_for(true).await;
        let target = create(&state, "organization-a", &request("Mine"), None)
            .await
            .unwrap();

        assert_eq!(list(&state, "organization-a").await.unwrap().len(), 1);
        assert!(list(&state, "organization-b").await.unwrap().is_empty());
        assert!(
            list_meta(&state, "organization-b")
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            get(&state, "organization-b", &target.id)
                .await
                .unwrap()
                .is_none()
        );
        let update_request = UpdateS3TargetRequest {
            name: "Stolen".into(),
            endpoint_url: "https://s3.example.com".into(),
            bucket: "bucket".into(),
            region: "us-east-1".into(),
            access_key_id: "AKIA".into(),
            secret_access_key: None,
        };
        assert!(matches!(
            update(&state, "organization-b", &target.id, &update_request).await,
            Err(AppError::ResourceNotFound)
        ));
        assert!(matches!(
            delete(&state, "organization-b", &target.id).await,
            Err(AppError::ResourceNotFound)
        ));
        assert!(
            resolve(&state, "organization-b", Some(&target.id))
                .await
                .is_err()
        );
        // Still there, unchanged, for its owner.
        assert_eq!(
            get(&state, "organization-a", &target.id)
                .await
                .unwrap()
                .unwrap()
                .name,
            "Mine"
        );
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn names_are_unique_per_organization_only() {
        let (state, path) = test_state_for(true).await;
        create(&state, "organization-a", &request("Backups"), None)
            .await
            .unwrap();
        create(&state, "organization-b", &request("Backups"), None)
            .await
            .unwrap();
        let duplicate = create(&state, "organization-a", &request("Backups"), None).await;
        assert!(
            matches!(duplicate, Err(AppError::ValidationError(_))),
            "{duplicate:?}"
        );
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_policy_cannot_point_at_another_organizations_target() {
        let (state, path) = test_state_for(true).await;
        let (_, _, app_b) = seed_application(&state, "organization-b").await;
        let target_a = create(&state, "organization-a", &request("A"), None)
            .await
            .unwrap();
        let refused = crate::database_backup::run::set_policy(
            &state,
            &app_b,
            &policy(false, Some(&target_a.id)),
            None,
        )
        .await;
        assert!(
            matches!(refused, Err(AppError::ValidationError(_))),
            "{refused:?}"
        );

        let target_b = create(&state, "organization-b", &request("B"), None)
            .await
            .unwrap();
        let saved = crate::database_backup::run::set_policy(
            &state,
            &app_b,
            &policy(true, Some(&target_b.id)),
            None,
        )
        .await
        .unwrap();
        assert_eq!(saved.s3_target_id.as_deref(), Some(target_b.id.as_str()));
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn cloud_never_falls_back_to_the_operators_bucket() {
        let (state, path) = test_state_for(true).await;
        let (_, _, app) = seed_application(&state, "organization-a").await;
        let refused =
            crate::database_backup::run::set_policy(&state, &app, &policy(true, None), None).await;
        assert!(
            matches!(refused, Err(AppError::ValidationError(_))),
            "{refused:?}"
        );
        let resolved = resolve(&state, "organization-a", None).await;
        assert!(
            matches!(&resolved, Err(AppError::ValidationError(message)) if message.contains("Choose an S3 target")),
            "{:?}",
            resolved.err()
        );
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn cloud_endpoints_must_be_public_https() {
        for refused in [
            "http://s3.example.com",
            "https://127.0.0.1:9000",
            "https://localhost:9000",
            "https://litepod-caddy:2019",
        ] {
            assert!(check_endpoint(true, refused).await.is_err(), "{refused}");
        }
        // Self-hosted keeps LAN storage such as MinIO.
        assert!(
            check_endpoint(false, "http://192.168.1.10:9000")
                .await
                .is_ok()
        );
    }
}
