use std::collections::HashMap;

use sqlx::Row;

use crate::{
    dtos::{deployment::EnvironmentVariableDto, response::AppError},
    state::state::SharedState,
    utils::security,
};

use super::resources::*;

pub(super) fn parse_environment(content: &str) -> Result<Vec<(String, String)>, AppError> {
    let mut values = HashMap::new();
    for (number, raw) in content.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            return Err(AppError::ValidationError(format!(
                "Environment line {} must use KEY=VALUE",
                number + 1
            )));
        };
        let valid_name = !name.is_empty()
            && name
                .bytes()
                .enumerate()
                .all(|(i, b)| b == b'_' || b.is_ascii_uppercase() || (i > 0 && b.is_ascii_digit()));
        if !valid_name {
            return Err(AppError::ValidationError(format!(
                "Environment line {} has an invalid variable name",
                number + 1
            )));
        }
        if values.insert(name.to_owned(), value.to_owned()).is_some() {
            return Err(AppError::ValidationError(format!(
                "Environment variable {name} is duplicated"
            )));
        }
    }
    Ok(values.into_iter().collect())
}

pub(crate) fn validate_environment_content(content: &str) -> Result<(), AppError> {
    parse_environment(content).map(|_| ())
}

pub(crate) fn parse_environment_content(content: &str) -> Result<Vec<(String, String)>, AppError> {
    parse_environment(content)
}

pub async fn environment(
    state: &SharedState,
    application_id: &str,
    scope: &str,
    reveal: bool,
) -> Result<Vec<EnvironmentVariableDto>, AppError> {
    if !matches!(scope, "runtime" | "build") {
        return Err(AppError::ValidationError(
            "Environment scope must be runtime or build".into(),
        ));
    }
    if reveal {
        return Ok(decrypted_environment(state, application_id, scope)
            .await?
            .into_iter()
            .map(|(name, value)| EnvironmentVariableDto {
                scope: scope.into(),
                name,
                value,
                masked: false,
            })
            .collect());
    }
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query("SELECT name FROM application_environment_variables WHERE application_id = ?1 AND scope = ?2 ORDER BY name")
        .bind(application_id)
        .bind(scope)
        .fetch_all(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut out = Vec::new();
    for row in &rows {
        out.push(EnvironmentVariableDto {
            scope: scope.into(),
            name: row
                .try_get(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
            value: MASK.into(),
            masked: true,
        });
    }
    Ok(out)
}

pub async fn save_environment(
    state: &SharedState,
    application_id: &str,
    scope: &str,
    content: &str,
) -> Result<Vec<EnvironmentVariableDto>, AppError> {
    if !matches!(scope, "runtime" | "build") {
        return Err(AppError::ValidationError(
            "Environment scope must be runtime or build".into(),
        ));
    }
    let parsed = parse_environment(content)?;
    crate::git::repositories::db::retry_on_stale_snapshot(|| {
        save_environment_once(state, application_id, scope, &parsed)
    })
    .await?;
    environment(state, application_id, scope, false).await
}

pub(super) async fn save_environment_once(
    state: &SharedState,
    application_id: &str,
    scope: &str,
    parsed: &[(String, String)],
) -> Result<(), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let mut tx = conn
        .begin()
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let old = sqlx::query("SELECT name FROM application_environment_variables WHERE application_id = ?1 AND scope = ?2")
        .bind(application_id)
        .bind(scope)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut existing = std::collections::HashSet::new();
    for row in &old {
        existing.insert(
            row.try_get::<String, _>(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?,
        );
    }
    let key = security::get_encryption_key();
    for (name, value) in parsed {
        if value == MASK && existing.contains(name) {
            existing.remove(name);
            continue;
        }
        let encrypted = security::encrypt_data(value, &key).map_err(AppError::InternalError)?;
        sqlx::query("INSERT INTO application_environment_variables (application_id, scope, name, encrypted_value, updated_at) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(application_id, scope, name) DO UPDATE SET encrypted_value = excluded.encrypted_value, updated_at = excluded.updated_at")
            .bind(application_id)
            .bind(scope)
            .bind(name.clone())
            .bind(encrypted)
            .bind(now_ms())
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        existing.remove(name);
    }
    for name in existing {
        sqlx::query("DELETE FROM application_environment_variables WHERE application_id = ?1 AND scope = ?2 AND name = ?3")
            .bind(application_id)
            .bind(scope)
            .bind(name)
            .execute(&mut *tx)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    }
    tx.commit()
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(())
}

/// Upserts one variable, leaving every other variable untouched.
///
/// [`save_environment`] takes the whole `.env` document and deletes anything
/// missing from it, which is right for the editor and wrong for a single
/// change: the assistant proposes one key, and a document round trip would let
/// a stale read silently drop the rest.
pub async fn set_environment_variable(
    state: &SharedState,
    application_id: &str,
    scope: &str,
    name: &str,
    value: &str,
) -> Result<(), AppError> {
    if !matches!(scope, "runtime" | "build") {
        return Err(AppError::ValidationError(
            "Environment scope must be runtime or build".into(),
        ));
    }
    if !is_environment_name(name) {
        return Err(AppError::ValidationError(format!(
            "`{name}` is not a valid environment variable name"
        )));
    }
    // Same expansion the environment editor does, so a `{{password_32}}` sent
    // by the assistant becomes a real secret instead of reaching a container.
    let value = crate::templates::secrets::substitute_value(value);
    let value = value.as_str();
    let variable = [(name.to_owned(), value.to_owned())];
    if scope == "runtime" {
        crate::deployment::compose::marker_routes::validate_env(state, application_id, &variable)
            .await?;
    } else if crate::deployment::compose::domain_markers::has_markers(&variable) {
        // Build variables are never resolved, so a marker would reach the build literally.
        return Err(AppError::ValidationError(
            "Domain markers ({{url}}, {{domain}}) only work in runtime variables".into(),
        ));
    }
    let key = security::get_encryption_key();
    let encrypted = security::encrypt_data(value, &key).map_err(AppError::InternalError)?;
    crate::git::repositories::db::retry_on_stale_snapshot(|| {
        let encrypted = encrypted.clone();
        async move {
            let conn = crate::git::repositories::db::get_conn(state).await;
            sqlx::query("INSERT INTO application_environment_variables (application_id, scope, name, encrypted_value, updated_at) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(application_id, scope, name) DO UPDATE SET encrypted_value = excluded.encrypted_value, updated_at = excluded.updated_at")
                .bind(application_id)
                .bind(scope)
                .bind(name)
                .bind(encrypted)
                .bind(now_ms())
                .execute(&conn)
                .await
                .map_err(|e| AppError::DatabaseError(e.to_string()))?;
            Ok(())
        }
    })
    .await
}

/// Removes one variable, leaving every other variable untouched.
///
/// Returns whether the row existed: a delete that matched nothing is not an
/// error, but the caller needs to know so it can say "there was nothing to
/// remove" instead of "removed".
pub async fn delete_environment_variable(
    state: &SharedState,
    application_id: &str,
    scope: &str,
    name: &str,
) -> Result<bool, AppError> {
    if !matches!(scope, "runtime" | "build") {
        return Err(AppError::ValidationError(
            "Environment scope must be runtime or build".into(),
        ));
    }
    crate::git::repositories::db::retry_on_stale_snapshot(|| async {
        let conn = crate::git::repositories::db::get_conn(state).await;
        let result = sqlx::query("DELETE FROM application_environment_variables WHERE application_id = ?1 AND scope = ?2 AND name = ?3")
            .bind(application_id)
            .bind(scope)
            .bind(name)
            .execute(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    })
    .await
}

/// The shape a container runtime accepts: a leading upper-case letter or
/// underscore, then upper-case letters, digits and underscores.
pub fn is_environment_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_uppercase() || first == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Decrypted KEY=VALUE pairs for container creation.
pub(crate) async fn decrypted_environment(
    state: &SharedState,
    application_id: &str,
    scope: &str,
) -> Result<Vec<(String, String)>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let variable_rows = sqlx::query("SELECT name, encrypted_value FROM application_environment_variables WHERE application_id = ?1 AND scope = ?2 ORDER BY name")
        .bind(application_id)
        .bind(scope)
        .fetch_all(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let key = security::get_encryption_key();
    let mut environment = Vec::new();
    for variable in &variable_rows {
        let name: String = variable
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let encrypted: String = variable
            .try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let value = security::decrypt_data(&encrypted, &key).map_err(AppError::InternalError)?;
        environment.push((name, value));
    }
    Ok(environment)
}
