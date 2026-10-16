use sqlx::Row;

use crate::state::state::SharedState;

pub struct UserRow {
    pub id: String,
    pub email: String,
    pub hashed_password: String,
    pub name: String,
    pub status_id: i64,
    pub created_at: String,
    pub email_verification_expires_at: Option<String>,
}

fn user_row_from(row: &sqlx::sqlite::SqliteRow) -> UserRow {
    UserRow {
        id: row.try_get(0).unwrap(),
        email: row.try_get(1).unwrap(),
        hashed_password: row.try_get(2).unwrap(),
        name: row.try_get(3).unwrap(),
        status_id: row.try_get(4).unwrap(),
        created_at: row.try_get(5).unwrap(),
        email_verification_expires_at: row.try_get(6).unwrap(),
    }
}

pub async fn create_user(
    state: &SharedState,
    id: &str,
    email: &str,
    hashed_password: &str,
    name: &str,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("INSERT INTO users (id, email, hashed_password, name) VALUES (?1, ?2, ?3, ?4)")
        .bind(id)
        .bind(email)
        .bind(hashed_password)
        .bind(name)
        .execute(&conn)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn create_pending_cloud_user(
    state: &SharedState,
    id: &str,
    email: &str,
    hashed_password: &str,
    name: &str,
    expires_at: &str,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO users (id, email, hashed_password, name, status_id, email_verification_expires_at) VALUES (?1, ?2, ?3, ?4, 3, ?5)",
    )
    .bind(id)
    .bind(email)
    .bind(hashed_password)
    .bind(name)
    .bind(expires_at)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn update_email_verification_expiry(
    state: &SharedState,
    user_id: &str,
    expires_at: &str,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE users SET email_verification_expires_at = ?1 WHERE id = ?2 AND status_id = 3",
    )
    .bind(expires_at)
    .bind(user_id)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn list_expired_pending_user_ids(
    state: &SharedState,
    now: &str,
) -> Result<Vec<String>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT id FROM users WHERE status_id = 3 AND email_verification_expires_at <= ?1 LIMIT 100",
    )
    .bind(now)
    .fetch_all(&conn)
    .await
    .map_err(|e| e.to_string())?;
    let mut ids = Vec::new();
    for row in &rows {
        ids.push(row.try_get(0).map_err(|e| e.to_string())?);
    }
    Ok(ids)
}

pub async fn delete_pending_user(state: &SharedState, user_id: &str) -> Result<bool, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    Ok(
        sqlx::query("DELETE FROM users WHERE id = ?1 AND status_id = 3")
            .bind(user_id)
            .execute(&conn)
            .await
            .map_err(|e| e.to_string())?
            .rows_affected()
            > 0,
    )
}

pub fn is_active(row: &UserRow) -> bool {
    row.status_id == 1
}

pub async fn get_user_by_email(state: &SharedState, email: &str) -> Option<UserRow> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT id, email, hashed_password, name, status_id, created_at, email_verification_expires_at FROM users WHERE email = ?1",
    )
    .bind(email)
    .fetch_optional(&conn)
    .await
    .ok()?;

    row.map(|row| user_row_from(&row))
}

pub async fn get_user_by_name(state: &SharedState, name: &str) -> Option<UserRow> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT id, email, hashed_password, name, status_id, created_at, email_verification_expires_at FROM users WHERE name = ?1",
    )
    .bind(name)
    .fetch_optional(&conn)
    .await
    .ok()?;

    row.map(|row| user_row_from(&row))
}

pub async fn get_user_by_id(state: &SharedState, user_id: &str) -> Option<UserRow> {
    match get_user_by_id_result(state, user_id).await {
        Ok(row) => row,
        Err(error) => {
            tracing::warn!(error = %error, user_id = %user_id, "user lookup failed");
            None
        }
    }
}

/// Result-returning variant: distinguishes "no row" (Ok(None)) from a database
/// error (Err). The auth middleware uses this so a transient DB outage maps
/// to 503 instead of being misreported as 401 and kicking the user out.
pub async fn get_user_by_id_result(
    state: &SharedState,
    user_id: &str,
) -> Result<Option<UserRow>, crate::dtos::response::AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT id, email, hashed_password, name, status_id, created_at, email_verification_expires_at FROM users WHERE id = ?1",
    )
    .bind(user_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| crate::dtos::response::AppError::DatabaseError(e.to_string()))?;

    Ok(row.map(|row| user_row_from(&row)))
}

pub async fn list_all_users(state: &SharedState) -> Result<Vec<UserRow>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT id, email, hashed_password, name, status_id, created_at, email_verification_expires_at FROM users ORDER BY created_at DESC",
    )
    .fetch_all(&conn)
    .await
    .map_err(|e| e.to_string())?;

    let mut users = Vec::new();
    for row in &rows {
        users.push(user_row_from(row));
    }
    Ok(users)
}

pub async fn update_user(
    state: &SharedState,
    user_id: &str,
    name: Option<&str>,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    if let Some(name) = name {
        sqlx::query("UPDATE users SET name = ?1 WHERE id = ?2")
            .bind(name)
            .bind(user_id)
            .execute(&conn)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub async fn delete_user(state: &SharedState, user_id: &str) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("DELETE FROM users WHERE id = ?1")
        .bind(user_id)
        .execute(&conn)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Disables (status 4) or re-enables an active user. Disabling also drops every
/// session; personal access tokens stay but the auth middleware refuses them
/// while the user is not active.
/// Pending and self-deactivated users are left alone.
pub async fn set_disabled(
    state: &SharedState,
    user_id: &str,
    disabled: bool,
) -> Result<bool, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let mut tx = conn.begin().await.map_err(|e| e.to_string())?;
    let (from, to) = if disabled { (1, 4) } else { (4, 1) };
    let changed = sqlx::query("UPDATE users SET status_id = ?1 WHERE id = ?2 AND status_id = ?3")
        .bind(to)
        .bind(user_id)
        .bind(from)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?
        .rows_affected()
        > 0;
    if changed && disabled {
        sqlx::query("DELETE FROM sessions WHERE user_id = ?1")
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(changed)
}

/// Grants the SysAdmin role only while the instance has none.
///
/// The `WHERE NOT EXISTS` makes the check and the write one statement, so two
/// concurrent registrations cannot both observe an empty table and both insert.
/// `Ok(false)` means a SysAdmin already exists and nothing was granted.
pub async fn make_sysadmin(state: &SharedState, user_id: &str) -> Result<bool, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let inserted = sqlx::query(
        "INSERT INTO user_global_roles (user_id, role_id)
         SELECT ?1, 0
         WHERE NOT EXISTS (SELECT 1 FROM user_global_roles WHERE role_id = 0)",
    )
    .bind(user_id)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();
    Ok(inserted > 0)
}

pub async fn count_users(state: &SharedState) -> Result<i64, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT COUNT(*) as cnt FROM users")
        .fetch_one(&conn)
        .await
        .map_err(|e| e.to_string())?;

    row.try_get::<i64, _>(0).map_err(|e| e.to_string())
}

pub async fn count_sysadmins(state: &SharedState) -> Result<i64, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT COUNT(*) FROM user_global_roles WHERE role_id = 0")
        .fetch_one(&conn)
        .await
        .map_err(|e| e.to_string())?;
    row.try_get(0).map_err(|e| e.to_string())
}

/// Optional cloud lock: when set, only this user id counts as SysAdmin, on top
/// of the database role. A second check the database alone cannot be talked out of.
pub const CLOUD_SYSADMIN_USER_ID_ENV: &str = "CLOUD_SYSADMIN_USER_ID";

pub async fn is_sysadmin(state: &SharedState, user_id: &str) -> bool {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT 1 FROM user_global_roles WHERE user_id = ?1 AND role_id = 0")
        .bind(user_id)
        .fetch_optional(&conn)
        .await;

    let has_role = matches!(row, Ok(Some(_)));
    let pinned = std::env::var(CLOUD_SYSADMIN_USER_ID_ENV).ok();
    has_role && passes_cloud_pin(state.is_cloud, user_id, pinned.as_deref())
}

fn passes_cloud_pin(is_cloud: bool, user_id: &str, pinned: Option<&str>) -> bool {
    match pinned.map(str::trim).filter(|id| !id.is_empty()) {
        Some(id) if is_cloud => user_id == id,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{cleanup_database, test_state};

    #[test]
    fn cloud_pin_only_narrows_sysadmin_on_cloud_when_set() {
        assert!(
            passes_cloud_pin(true, "admin", None),
            "unset: database role alone"
        );
        assert!(
            passes_cloud_pin(true, "admin", Some("  ")),
            "blank counts as unset"
        );
        assert!(passes_cloud_pin(true, "admin", Some("admin")));
        assert!(!passes_cloud_pin(true, "admin", Some("someone-else")));
        assert!(
            passes_cloud_pin(false, "admin", Some("someone-else")),
            "self-hosted ignores it"
        );
    }

    #[tokio::test]
    async fn set_disabled_blocks_login_and_drops_sessions() {
        let (state, path) = test_state().await;
        let conn = crate::git::repositories::db::get_conn(&state).await;
        sqlx::query("INSERT INTO users (id, email, hashed_password, name, status_id) VALUES ('u', 'u@x.test', 'h', 'U', 1)")
            .execute(&conn).await.unwrap();
        sqlx::query("INSERT INTO sessions (id, user_id, token_digest, expires_at) VALUES ('s', 'u', 'd', '2099-01-01 00:00:00')")
            .execute(&conn).await.unwrap();

        assert!(set_disabled(&state, "u", true).await.unwrap());
        assert!(!is_active(&get_user_by_id(&state, "u").await.unwrap()));
        let sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE user_id = 'u'")
            .fetch_one(&conn)
            .await
            .unwrap();
        assert_eq!(sessions, 0);
        // Already disabled: no-op.
        assert!(!set_disabled(&state, "u", true).await.unwrap());

        assert!(set_disabled(&state, "u", false).await.unwrap());
        assert!(is_active(&get_user_by_id(&state, "u").await.unwrap()));

        cleanup_database(&path);
    }
}
