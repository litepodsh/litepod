//! Per-application HTTP Basic Authentication.
//!
//! One row in `application_basic_auth` per application that has touched the
//! feature; zero or more rows in `application_basic_auth_users`; zero or more
//! rows in `application_basic_auth_domains`. The Caddy route for a domain gets
//! an `authentication` handler prepended iff:
//!
//! - the row exists,
//! - `enabled = true`,
//! - the domain id is present in `application_basic_auth_domains`, and
//! - at least one user exists (Caddy refuses to load a basicauth route with an
//!   empty `accounts` array, so the API returns a 422 before that).
//!
//! Passwords are bcrypt-hashed (`bcrypt = "0.16"`), the canonical form for
//! Caddy's `http_basic` provider, and never leave the server again. The
//! `BasicAuthUserDto` reports only `has_password: bool` so the UI can show
//! "Set password" without ever rendering the hash.

use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    dtos::{
        deployment::{BasicAuthConfigDto, BasicAuthUserDto, UpdateBasicAuthRequest},
        response::AppError,
    },
    state::state::SharedState,
};

/// Bcrypt cost factor. Caddy reads this back to verify the hash, so it must
/// match what `bcrypt::hash` produced. Cost 12 ≈ 250ms on a single core — the
/// same ballpark as `utils::security::hash_password` (Argon2id).
const BCRYPT_COST: u32 = 12;

const MIN_PASSWORD_LEN: usize = 8;
const MAX_PASSWORD_LEN: usize = 256;
const MAX_USERNAME_LEN: usize = 64;
const MAX_REALM_LEN: usize = 64;

#[derive(Debug, Clone)]
pub(crate) struct BasicAuthPayload {
    pub realm: String,
    pub accounts: Vec<(String, String)>,
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn validate_username(username: &str) -> Result<(), AppError> {
    if username.is_empty()
        || username.len() > MAX_USERNAME_LEN
        || !username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(AppError::ValidationError(
            "Username must be 1-64 chars of letters, digits, dot, underscore or dash".into(),
        ));
    }
    Ok(())
}

fn validate_password(password: &str) -> Result<(), AppError> {
    if password.len() < MIN_PASSWORD_LEN {
        return Err(AppError::ValidationError(format!(
            "Password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    if password.len() > MAX_PASSWORD_LEN {
        return Err(AppError::ValidationError(format!(
            "Password must be at most {MAX_PASSWORD_LEN} characters"
        )));
    }
    Ok(())
}

fn validate_realm(realm: &str) -> Result<(), AppError> {
    if realm.is_empty()
        || realm.len() > MAX_REALM_LEN
        || realm.contains('\n')
        || realm.contains('\r')
    {
        return Err(AppError::ValidationError(format!(
            "Realm must be 1-{MAX_REALM_LEN} characters with no newlines"
        )));
    }
    Ok(())
}

fn row_to_user(row: &sqlx::sqlite::SqliteRow) -> Result<BasicAuthUserDto, AppError> {
    let text = |i: usize| -> Result<String, AppError> {
        row.try_get(i)
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    };
    let int = |i: usize| -> Result<i64, AppError> {
        row.try_get(i)
            .map_err(|e| AppError::DatabaseError(e.to_string()))
    };
    let hash: String = text(2)?;
    Ok(BasicAuthUserDto {
        id: text(0)?,
        username: text(1)?,
        has_password: !hash.is_empty(),
        created_at: int(3)?,
        updated_at: int(4)?,
    })
}

/// Returns the application-scoped `auth_id`, creating an empty row if none
/// exists yet so the rest of the write path can FK against a stable id.
async fn ensure_auth_id(state: &SharedState, application_id: &str) -> Result<String, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    if let Some(row) =
        sqlx::query("SELECT id FROM application_basic_auth WHERE application_id = ?1")
            .bind(application_id)
            .fetch_optional(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?
    {
        let id: String = row
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        return Ok(id);
    }
    let id = Uuid::now_v7().to_string();
    let now = now_ms();
    sqlx::query(
        "INSERT INTO application_basic_auth (id, application_id, enabled, realm, created_at, updated_at) \
         VALUES (?1, ?2, 0, 'Restricted', ?3, ?3)",
    )
    .bind(&id)
    .bind(application_id)
    .bind(now)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(id)
}

async fn fetch_users(
    state: &SharedState,
    auth_id: &str,
) -> Result<Vec<BasicAuthUserDto>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT id, username, password_hash, created_at, updated_at \
         FROM application_basic_auth_users WHERE auth_id = ?1 ORDER BY username",
    )
    .bind(auth_id)
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    rows.iter().map(row_to_user).collect()
}

async fn fetch_protected_domain_ids(
    state: &SharedState,
    auth_id: &str,
) -> Result<Vec<String>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT domain_id FROM application_basic_auth_domains WHERE auth_id = ?1 ORDER BY domain_id",
    )
    .bind(auth_id)
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    rows.iter()
        .map(|row| {
            row.try_get::<String, _>(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))
        })
        .collect()
}

async fn fetch_enabled_and_realm(
    state: &SharedState,
    auth_id: &str,
) -> Result<(bool, String), AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT enabled, realm FROM application_basic_auth WHERE id = ?1")
        .bind(auth_id)
        .fetch_one(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let enabled: i64 = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let realm: String = row
        .try_get(1)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok((enabled != 0, realm))
}

/// Full read of the config for one application. Returns a default-shaped DTO
/// (enabled=false, realm="Restricted", empty users/domains) when no row exists
/// yet — the Advanced tab renders cleanly on first visit.
pub async fn get(
    state: &SharedState,
    application_id: &str,
) -> Result<BasicAuthConfigDto, AppError> {
    let auth_id = ensure_auth_id(state, application_id).await?;
    let (enabled, realm) = fetch_enabled_and_realm(state, &auth_id).await?;
    let users = fetch_users(state, &auth_id).await?;
    let protected_domain_ids = fetch_protected_domain_ids(state, &auth_id).await?;
    Ok(BasicAuthConfigDto {
        enabled,
        realm,
        users,
        protected_domain_ids,
    })
}

/// Updates enabled/realm and the protected-domain set. Re-pushes the Caddy
/// route for every domain whose membership changed.
pub async fn update(
    state: &SharedState,
    application_id: &str,
    request: UpdateBasicAuthRequest,
) -> Result<BasicAuthConfigDto, AppError> {
    if let Some(realm) = request.realm.as_deref() {
        validate_realm(realm)?;
    }
    let auth_id = ensure_auth_id(state, application_id).await?;
    let previous = fetch_protected_domain_ids(state, &auth_id).await?;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let now = now_ms();
    if request.enabled.is_some() || request.realm.is_some() {
        let mut sql = String::from("UPDATE application_basic_auth SET updated_at = ?");
        if request.enabled.is_some() {
            sql.push_str(", enabled = ?");
        }
        if request.realm.is_some() {
            sql.push_str(", realm = ?");
        }
        sql.push_str(" WHERE id = ?");
        let mut query = sqlx::query(&sql).bind(now);
        if let Some(enabled) = request.enabled {
            query = query.bind(enabled as i64);
        }
        if let Some(realm) = request.realm.as_deref() {
            query = query.bind(realm);
        }
        query = query.bind(&auth_id);
        query
            .execute(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    }
    let mut next_ids: Vec<String> = Vec::new();
    if let Some(ids) = request.protected_domain_ids.as_ref() {
        // Reject any id that does not belong to this application; otherwise a
        // caller could attach auth to somebody else's domain.
        let placeholders = std::iter::repeat_n("?", ids.len().max(1))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT id FROM application_domains WHERE application_id = ? AND id IN ({placeholders})",
        );
        let mut query = sqlx::query(&sql).bind(application_id);
        for id in ids {
            query = query.bind(id);
        }
        let rows = query
            .fetch_all(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        for row in &rows {
            let id: String = row
                .try_get(0)
                .map_err(|e| AppError::DatabaseError(e.to_string()))?;
            next_ids.push(id);
        }
        sqlx::query("DELETE FROM application_basic_auth_domains WHERE auth_id = ?1")
            .bind(&auth_id)
            .execute(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        for domain_id in &next_ids {
            sqlx::query(
                "INSERT OR IGNORE INTO application_basic_auth_domains (auth_id, domain_id) VALUES (?1, ?2)",
            )
            .bind(&auth_id)
            .bind(domain_id)
            .execute(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        }
    } else {
        next_ids = previous.clone();
    }
    drop(conn);

    // Reconcile Caddy for the symmetric difference between previous and next.
    let mut all: Vec<String> = previous.clone();
    for id in &next_ids {
        if !all.contains(id) {
            all.push(id.clone());
        }
    }
    for domain_id in all {
        let membership_changed = previous.contains(&domain_id) != next_ids.contains(&domain_id);
        if membership_changed {
            super::reconcile_caddy_basicauth(state, &domain_id).await;
        }
    }
    let (enabled, realm) = fetch_enabled_and_realm(state, &auth_id).await?;
    let users = fetch_users(state, &auth_id).await?;
    Ok(BasicAuthConfigDto {
        enabled,
        realm,
        users,
        protected_domain_ids: next_ids,
    })
}

pub async fn add_user(
    state: &SharedState,
    application_id: &str,
    username: String,
    password: String,
) -> Result<BasicAuthUserDto, AppError> {
    validate_username(&username)?;
    validate_password(&password)?;
    let auth_id = ensure_auth_id(state, application_id).await?;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let existing = sqlx::query(
        "SELECT id FROM application_basic_auth_users WHERE auth_id = ?1 AND username = ?2",
    )
    .bind(&auth_id)
    .bind(&username)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    if existing.is_some() {
        return Err(AppError::ValidationError(format!(
            "User {username} already exists"
        )));
    }
    let hash = bcrypt::hash(&password, BCRYPT_COST)
        .map_err(|e| AppError::InternalError(format!("bcrypt hash failed: {e}")))?;
    let id = Uuid::now_v7().to_string();
    let now = now_ms();
    sqlx::query(
        "INSERT INTO application_basic_auth_users (id, auth_id, username, password_hash, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
    )
    .bind(&id)
    .bind(&auth_id)
    .bind(&username)
    .bind(&hash)
    .bind(now)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    let user = sqlx::query(
        "SELECT id, username, password_hash, created_at, updated_at FROM application_basic_auth_users WHERE id = ?1",
    )
    .bind(&id)
    .fetch_one(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let dto = row_to_user(&user)?;
    drop(conn);

    // A new user might unblock protected domains that had no users before.
    for domain_id in fetch_protected_domain_ids(state, &auth_id).await? {
        super::reconcile_caddy_basicauth(state, &domain_id).await;
    }
    Ok(dto)
}

pub async fn update_user(
    state: &SharedState,
    application_id: &str,
    user_id: &str,
    new_username: Option<String>,
    new_password: Option<String>,
) -> Result<BasicAuthUserDto, AppError> {
    if new_username.is_none() && new_password.is_none() {
        return Err(AppError::ValidationError(
            "Provide a username or password to update".into(),
        ));
    }
    if let Some(username) = new_username.as_deref() {
        validate_username(username)?;
    }
    if let Some(password) = new_password.as_deref() {
        validate_password(password)?;
    }
    let auth_id = ensure_auth_id(state, application_id).await?;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let existing = sqlx::query(
        "SELECT id, username, password_hash, created_at, updated_at FROM application_basic_auth_users \
         WHERE id = ?1 AND auth_id = ?2",
    )
    .bind(user_id)
    .bind(&auth_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let Some(row) = existing else {
        return Err(AppError::ResourceNotFound);
    };
    let row_username: String = row
        .try_get(1)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    if let Some(username) = new_username.as_deref() {
        if username != row_username {
            let clash = sqlx::query(
                "SELECT id FROM application_basic_auth_users WHERE auth_id = ?1 AND username = ?2 AND id != ?3",
            )
            .bind(&auth_id)
            .bind(username)
            .bind(user_id)
            .fetch_optional(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
            if clash.is_some() {
                return Err(AppError::ValidationError(format!(
                    "User {username} already exists"
                )));
            }
        }
    }
    let final_username = new_username.clone().unwrap_or(row_username);
    let new_hash = match new_password.as_deref() {
        Some(password) => Some(
            bcrypt::hash(password, BCRYPT_COST)
                .map_err(|e| AppError::InternalError(format!("bcrypt hash failed: {e}")))?,
        ),
        None => None,
    };
    if let Some(hash) = new_hash.as_deref() {
        sqlx::query(
            "UPDATE application_basic_auth_users SET username = ?2, password_hash = ?3, updated_at = ?4 \
             WHERE id = ?1 AND auth_id = ?5",
        )
        .bind(user_id)
        .bind(&final_username)
        .bind(hash)
        .bind(now_ms())
        .bind(&auth_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    } else {
        sqlx::query(
            "UPDATE application_basic_auth_users SET username = ?2, updated_at = ?3 \
             WHERE id = ?1 AND auth_id = ?4",
        )
        .bind(user_id)
        .bind(&final_username)
        .bind(now_ms())
        .bind(&auth_id)
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    }
    let refreshed = sqlx::query(
        "SELECT id, username, password_hash, created_at, updated_at FROM application_basic_auth_users WHERE id = ?1",
    )
    .bind(user_id)
    .fetch_one(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let dto = row_to_user(&refreshed)?;
    drop(conn);

    for domain_id in fetch_protected_domain_ids(state, &auth_id).await? {
        super::reconcile_caddy_basicauth(state, &domain_id).await;
    }
    Ok(dto)
}

pub async fn delete_user(
    state: &SharedState,
    application_id: &str,
    user_id: &str,
) -> Result<(), AppError> {
    let auth_id = ensure_auth_id(state, application_id).await?;
    let conn = crate::git::repositories::db::get_conn(state).await;
    let result =
        sqlx::query("DELETE FROM application_basic_auth_users WHERE id = ?1 AND auth_id = ?2")
            .bind(user_id)
            .bind(&auth_id)
            .execute(&conn)
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    drop(conn);
    if result.rows_affected() == 0 {
        return Err(AppError::ResourceNotFound);
    }
    for domain_id in fetch_protected_domain_ids(state, &auth_id).await? {
        super::reconcile_caddy_basicauth(state, &domain_id).await;
    }
    Ok(())
}

/// Loads the accounts Caddy needs to render the `authentication` handler for
/// the given domain. Returns `None` when basicauth is disabled for the domain
/// or no users exist (so the caller emits just the reverse_proxy handle).
pub(crate) async fn load_payload_for_domain(
    state: &SharedState,
    domain_id: &str,
) -> Result<Option<BasicAuthPayload>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT ab.enabled, ab.realm \
         FROM application_basic_auth ab \
         JOIN application_basic_auth_domains bd ON bd.auth_id = ab.id \
         WHERE bd.domain_id = ?1",
    )
    .bind(domain_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let enabled: i64 = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    if enabled == 0 {
        return Ok(None);
    }
    let realm: String = row
        .try_get(1)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let account_rows = sqlx::query(
        "SELECT u.username, u.password_hash \
         FROM application_basic_auth_users u \
         JOIN application_basic_auth_domains bd ON bd.auth_id = u.auth_id \
         WHERE bd.domain_id = ?1 \
         ORDER BY u.username",
    )
    .bind(domain_id)
    .fetch_all(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let mut accounts = Vec::with_capacity(account_rows.len());
    for row in &account_rows {
        let username: String = row
            .try_get(0)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let password_hash: String = row
            .try_get(1)
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        accounts.push((username, password_hash));
    }
    if accounts.is_empty() {
        // Caddy refuses an authentication handler with no accounts; leave the
        // route unprotected rather than wedge the load.
        return Ok(None);
    }
    Ok(Some(BasicAuthPayload { realm, accounts }))
}

pub(crate) fn authentication_handle(
    payload: &BasicAuthPayload,
) -> Result<serde_json::Value, AppError> {
    let accounts: Vec<serde_json::Value> = payload
        .accounts
        .iter()
        .map(|(username, password)| {
            serde_json::json!({ "username": username, "password": password })
        })
        .collect();
    Ok(serde_json::json!({
        "handler": "authentication",
        "providers": {
            "http_basic": {
                "accounts": accounts,
                "hash": { "algorithm": "bcrypt" },
                "realm": payload.realm
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authentication_realm_belongs_to_http_basic_provider() {
        let handle = authentication_handle(&BasicAuthPayload {
            realm: "Pastefy".into(),
            accounts: vec![("admin".into(), "$2a$12$hash".into())],
        })
        .unwrap();

        assert_eq!(handle["providers"]["http_basic"]["realm"], "Pastefy");
        assert!(handle.get("realm").is_none());
    }
}
