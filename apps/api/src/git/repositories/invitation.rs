use sqlx::Row;

use crate::state::state::SharedState;

#[derive(Debug, Clone)]
pub struct InvitationRow {
    pub id: String,
    pub user_id: String,
    pub username: String,
    pub email: String,
    pub created_by: String,
    pub created_by_name: String,
    #[allow(dead_code)] // surfaced via audit log; populated for symmetry with accept path.
    pub token_hash: String,
    pub expires_at: String,
    pub accepted_at: Option<String>,
    pub revoked_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct InvitationOrganizationRow {
    pub organization_id: String,
    pub organization_name: String,
    pub role_id: i64,
    pub role_name: String,
}

const INVITATION_COLUMNS: &str = "i.id, i.user_id, u.name, u.email, i.created_by, creator.name, i.token_hash, i.expires_at, i.accepted_at, i.revoked_at, i.created_at";

fn row_to_invitation(row: &sqlx::sqlite::SqliteRow) -> Result<InvitationRow, String> {
    Ok(InvitationRow {
        id: row.try_get(0).map_err(|e| e.to_string())?,
        user_id: row.try_get(1).map_err(|e| e.to_string())?,
        username: row.try_get(2).map_err(|e| e.to_string())?,
        email: row.try_get(3).map_err(|e| e.to_string())?,
        created_by: row.try_get(4).map_err(|e| e.to_string())?,
        created_by_name: row.try_get(5).map_err(|e| e.to_string())?,
        token_hash: row.try_get(6).map_err(|e| e.to_string())?,
        expires_at: row.try_get(7).map_err(|e| e.to_string())?,
        accepted_at: row.try_get(8).map_err(|e| e.to_string())?,
        revoked_at: row.try_get(9).map_err(|e| e.to_string())?,
        created_at: row.try_get(10).map_err(|e| e.to_string())?,
    })
}

/// Only ever called inside a transaction, so it takes a generic executor
/// rather than a concrete connection type.
pub async fn insert<'e, E>(
    executor: E,
    id: &str,
    user_id: &str,
    created_by: &str,
    token_hash: &str,
) -> Result<(), String>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "INSERT INTO invitations (id, user_id, created_by, token_hash, expires_at)
         VALUES (?1, ?2, ?3, ?4, datetime('now', '+7 days'))",
    )
    .bind(id)
    .bind(user_id)
    .bind(created_by)
    .bind(token_hash)
    .execute(executor)
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

pub async fn find_by_token_hash(
    state: &SharedState,
    token_hash: &str,
) -> Result<Option<InvitationRow>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    find_by_token_hash_on(&conn, token_hash).await
}

/// Called both standalone (with the pool) and inside a transaction (when
/// accepting an invitation), so it takes a generic executor.
pub async fn find_by_token_hash_on<'e, E>(
    executor: E,
    token_hash: &str,
) -> Result<Option<InvitationRow>, String>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let row = sqlx::query(&format!(
        "SELECT {INVITATION_COLUMNS} FROM invitations i
         JOIN users u ON u.id = i.user_id
         JOIN users creator ON creator.id = i.created_by
         WHERE i.token_hash = ?1"
    ))
    .bind(token_hash)
    .fetch_optional(executor)
    .await
    .map_err(|e| e.to_string())?;
    match row {
        Some(row) => row_to_invitation(&row).map(Some),
        None => Ok(None),
    }
}

pub async fn find_by_id(state: &SharedState, id: &str) -> Result<Option<InvitationRow>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(&format!(
        "SELECT {INVITATION_COLUMNS} FROM invitations i
         JOIN users u ON u.id = i.user_id
         JOIN users creator ON creator.id = i.created_by
         WHERE i.id = ?1"
    ))
    .bind(id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| e.to_string())?;
    match row {
        Some(row) => row_to_invitation(&row).map(Some),
        None => Ok(None),
    }
}

pub async fn find_by_id_for_creator(
    state: &SharedState,
    id: &str,
    created_by: &str,
) -> Result<Option<InvitationRow>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(&format!(
        "SELECT {INVITATION_COLUMNS} FROM invitations i
         JOIN users u ON u.id = i.user_id
         JOIN users creator ON creator.id = i.created_by
         WHERE i.id = ?1 AND i.created_by = ?2"
    ))
    .bind(id)
    .bind(created_by)
    .fetch_optional(&conn)
    .await
    .map_err(|e| e.to_string())?;
    match row {
        Some(row) => row_to_invitation(&row).map(Some),
        None => Ok(None),
    }
}

pub async fn list(state: &SharedState) -> Result<Vec<InvitationRow>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {INVITATION_COLUMNS} FROM invitations i
         JOIN users u ON u.id = i.user_id
         JOIN users creator ON creator.id = i.created_by
         ORDER BY i.created_at DESC"
    ))
    .fetch_all(&conn)
    .await
    .map_err(|e| e.to_string())?;
    let mut invitations = Vec::new();
    for row in &rows {
        invitations.push(row_to_invitation(row)?);
    }
    Ok(invitations)
}

pub async fn list_for_creator(
    state: &SharedState,
    created_by: &str,
) -> Result<Vec<InvitationRow>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {INVITATION_COLUMNS} FROM invitations i
         JOIN users u ON u.id = i.user_id
         JOIN users creator ON creator.id = i.created_by
         WHERE i.created_by = ?1
         ORDER BY i.created_at DESC"
    ))
    .bind(created_by)
    .fetch_all(&conn)
    .await
    .map_err(|e| e.to_string())?;
    let mut invitations = Vec::new();
    for row in &rows {
        invitations.push(row_to_invitation(row)?);
    }
    Ok(invitations)
}

pub async fn list_organizations(
    state: &SharedState,
    user_id: &str,
) -> Result<Vec<InvitationOrganizationRow>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT t.id, t.name, tm.role_id, r.name
         FROM organization_members tm
         JOIN organizations t ON t.id = tm.organization_id
         JOIN roles r ON r.id = tm.role_id
         WHERE tm.user_id = ?1
         ORDER BY t.name COLLATE NOCASE ASC",
    )
    .bind(user_id)
    .fetch_all(&conn)
    .await
    .map_err(|e| e.to_string())?;
    let mut organizations = Vec::new();
    for row in &rows {
        organizations.push(InvitationOrganizationRow {
            organization_id: row.try_get(0).map_err(|e| e.to_string())?,
            organization_name: row.try_get(1).map_err(|e| e.to_string())?,
            role_id: row.try_get(2).map_err(|e| e.to_string())?,
            role_name: row.try_get(3).map_err(|e| e.to_string())?,
        });
    }
    Ok(organizations)
}

pub async fn revoke(state: &SharedState, id: &str) -> Result<u64, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE invitations SET revoked_at = datetime('now')
         WHERE id = ?1 AND accepted_at IS NULL AND revoked_at IS NULL",
    )
    .bind(id)
    .execute(&conn)
    .await
    .map(|result| result.rows_affected())
    .map_err(|e| e.to_string())
}

pub async fn revoke_for_creator(
    state: &SharedState,
    id: &str,
    created_by: &str,
) -> Result<u64, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE invitations SET revoked_at = datetime('now')
         WHERE id = ?1 AND created_by = ?2 AND accepted_at IS NULL AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(created_by)
    .execute(&conn)
    .await
    .map(|result| result.rows_affected())
    .map_err(|e| e.to_string())
}

pub async fn regenerate(state: &SharedState, id: &str, token_hash: &str) -> Result<u64, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE invitations
         SET token_hash = ?1, expires_at = datetime('now', '+7 days'), revoked_at = NULL
         WHERE id = ?2 AND accepted_at IS NULL",
    )
    .bind(token_hash)
    .bind(id)
    .execute(&conn)
    .await
    .map(|result| result.rows_affected())
    .map_err(|e| e.to_string())
}

pub async fn regenerate_for_creator(
    state: &SharedState,
    id: &str,
    created_by: &str,
    token_hash: &str,
) -> Result<u64, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE invitations
         SET token_hash = ?1, expires_at = datetime('now', '+7 days'), revoked_at = NULL
         WHERE id = ?2 AND created_by = ?3 AND accepted_at IS NULL",
    )
    .bind(token_hash)
    .bind(id)
    .bind(created_by)
    .execute(&conn)
    .await
    .map(|result| result.rows_affected())
    .map_err(|e| e.to_string())
}

/// Only ever called inside a transaction, so it takes a generic executor
/// rather than a concrete connection type.
pub async fn mark_accepted<'e, E>(executor: E, id: &str) -> Result<u64, String>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "UPDATE invitations SET accepted_at = datetime('now')
         WHERE id = ?1 AND accepted_at IS NULL AND revoked_at IS NULL AND expires_at > datetime('now')",
    )
    .bind(id)
    .execute(executor)
    .await
    .map(|result| result.rows_affected())
    .map_err(|e| e.to_string())
}
