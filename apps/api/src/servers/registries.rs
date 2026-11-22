//! Container registry logins saved once per organization. Build servers push with
//! them and deploy servers pull with them, matched by the image's registry host.

use chrono::Utc;
use sqlx::Row;

use crate::{
    dtos::{
        response::AppError,
        server::{RegistryCredentialDto, SaveRegistryCredentialRequest},
    },
    state::state::SharedState,
};

fn db_error(error: sqlx::Error) -> AppError {
    AppError::DatabaseError(error.to_string())
}

/// A registry host as it starts an image reference: `ghcr.io`, `registry.example.com:5000`
/// or `localhost:5000`. A bare word would be read as a Docker Hub namespace.
pub(crate) fn valid_registry_host(host: &str) -> bool {
    (host.contains('.') || host.contains(':') || host == "localhost")
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
}

pub async fn list(
    state: &SharedState,
    organization_id: &str,
) -> Result<Vec<RegistryCredentialDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT id, registry, username, updated_at FROM registry_credentials WHERE organization_id = ?1 ORDER BY registry",
    )
    .bind(organization_id)
    .fetch_all(&conn)
    .await
    .map_err(db_error)?;
    rows.iter()
        .map(|row| {
            Ok(RegistryCredentialDto {
                id: row.try_get(0).map_err(db_error)?,
                registry: row.try_get(1).map_err(db_error)?,
                username: row.try_get(2).map_err(db_error)?,
                updated_at: row.try_get(3).map_err(db_error)?,
            })
        })
        .collect()
}

/// Saves the login for a registry, replacing the one already saved for that host.
pub async fn save(
    state: &SharedState,
    organization_id: &str,
    request: SaveRegistryCredentialRequest,
) -> Result<RegistryCredentialDto, AppError> {
    let registry = request.registry.trim().to_ascii_lowercase();
    let username = request.username.trim();
    if !valid_registry_host(&registry) {
        return Err(AppError::ValidationError(
            "Registry must be a host such as ghcr.io or registry.example.com:5000".into(),
        ));
    }
    if username.is_empty() || request.password.is_empty() {
        return Err(AppError::ValidationError(
            "Registry username and password are required".into(),
        ));
    }
    let key = crate::utils::security::get_encryption_key();
    let encrypted = crate::utils::security::encrypt_data(&request.password, &key)
        .map_err(AppError::InternalError)?;
    let now = Utc::now().timestamp_millis();
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO registry_credentials (id, organization_id, registry, username, password_encrypted, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6) \
         ON CONFLICT (organization_id, registry) DO UPDATE SET username = excluded.username, password_encrypted = excluded.password_encrypted, updated_at = excluded.updated_at",
    )
    .bind(uuid::Uuid::now_v7().to_string())
    .bind(organization_id)
    .bind(&registry)
    .bind(username)
    .bind(encrypted)
    .bind(now)
    .execute(&conn)
    .await
    .map_err(db_error)?;
    list(state, organization_id)
        .await?
        .into_iter()
        .find(|credential| credential.registry == registry)
        .ok_or(AppError::ResourceNotFound)
}

pub async fn delete(state: &SharedState, organization_id: &str, id: &str) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let deleted =
        sqlx::query("DELETE FROM registry_credentials WHERE id = ?1 AND organization_id = ?2")
            .bind(id)
            .bind(organization_id)
            .execute(&conn)
            .await
            .map_err(db_error)?;
    if deleted.rows_affected() == 0 {
        return Err(AppError::ResourceNotFound);
    }
    Ok(())
}

/// The decrypted `(username, password)` saved for `registry`, `None` when the
/// organization has none (a public registry, or one that needs no login).
pub(crate) async fn for_registry(
    state: &SharedState,
    organization_id: &str,
    registry: &str,
) -> Result<Option<(String, String)>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let Some(row) = sqlx::query(
        "SELECT username, password_encrypted FROM registry_credentials WHERE organization_id = ?1 AND registry = ?2",
    )
    .bind(organization_id)
    .bind(registry.to_ascii_lowercase())
    .fetch_optional(&conn)
    .await
    .map_err(db_error)?
    else {
        return Ok(None);
    };
    let key = crate::utils::security::get_encryption_key();
    let password =
        crate::utils::security::decrypt_data(&row.try_get::<String, _>(1).map_err(db_error)?, &key)
            .map_err(AppError::InternalError)?;
    Ok(Some((row.try_get(0).map_err(db_error)?, password)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_host_needs_a_dot_a_port_or_localhost() {
        assert!(valid_registry_host("ghcr.io"));
        assert!(valid_registry_host("registry.example.com:5000"));
        assert!(valid_registry_host("localhost"));
        assert!(!valid_registry_host("team"));
        assert!(!valid_registry_host("ghcr.io/acme"));
        assert!(!valid_registry_host(""));
    }

    #[tokio::test]
    async fn saving_a_registry_again_replaces_its_login_and_stays_in_its_organization() {
        // `test_state` already seeds organization-a and organization-b.
        let (state, path) = crate::testing::test_state().await;
        let request = |username: &str, password: &str| SaveRegistryCredentialRequest {
            registry: "GHCR.io ".into(),
            username: username.into(),
            password: password.into(),
        };
        save(&state, "organization-a", request("robot", "first"))
            .await
            .unwrap();
        let saved = save(&state, "organization-a", request("robot2", "second"))
            .await
            .unwrap();
        assert_eq!(saved.registry, "ghcr.io");
        assert_eq!(list(&state, "organization-a").await.unwrap().len(), 1);
        assert_eq!(
            for_registry(&state, "organization-a", "ghcr.io")
                .await
                .unwrap(),
            Some(("robot2".into(), "second".into()))
        );
        assert_eq!(
            for_registry(&state, "organization-b", "ghcr.io")
                .await
                .unwrap(),
            None
        );
        assert!(delete(&state, "organization-b", &saved.id).await.is_err());
        delete(&state, "organization-a", &saved.id).await.unwrap();
        assert!(list(&state, "organization-a").await.unwrap().is_empty());
        crate::testing::cleanup_database(&path);
    }
}
