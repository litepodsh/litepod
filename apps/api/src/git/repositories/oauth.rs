//! Rows behind the MCP endpoint's OAuth (migration 0069). Codes and refresh
//! tokens are stored only as digests, like personal access tokens.

use sqlx::Row;

use crate::state::state::SharedState;
use crate::utils::security::digest_token as digest;

pub struct OAuthClient {
    pub id: String,
    pub name: String,
    pub redirect_uris: Vec<String>,
}

pub struct NewCode<'a> {
    pub code: &'a str,
    pub client_id: &'a str,
    pub user_id: &'a str,
    pub organization_id: &'a str,
    pub redirect_uri: &'a str,
    pub code_challenge: &'a str,
    pub expires_at: &'a str,
}

pub struct CodeRow {
    pub code_digest: String,
    pub client_id: String,
    pub user_id: String,
    pub organization_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub expires_at: String,
    /// Already exchanged before this call: a replay.
    pub used: bool,
}

pub struct NewRefresh<'a> {
    pub token: &'a str,
    pub client_id: &'a str,
    pub user_id: &'a str,
    pub organization_id: &'a str,
    pub access_token_id: &'a str,
    pub code_digest: &'a str,
    pub expires_at: &'a str,
}

pub struct RefreshRow {
    pub client_id: String,
    pub user_id: String,
    pub organization_id: String,
    pub access_token_id: String,
    pub code_digest: String,
    pub expires_at: String,
}

pub async fn create_client(
    state: &SharedState,
    id: &str,
    name: &str,
    redirect_uris_json: &str,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("INSERT INTO oauth_clients (id, name, redirect_uris) VALUES (?1, ?2, ?3)")
        .bind(id)
        .bind(name)
        .bind(redirect_uris_json)
        .execute(&conn)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn get_client(state: &SharedState, id: &str) -> Result<Option<OAuthClient>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT id, name, redirect_uris FROM oauth_clients WHERE id = ?1")
        .bind(id)
        .fetch_optional(&conn)
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.map(|row| OAuthClient {
        id: row.try_get(0).unwrap(),
        name: row.try_get(1).unwrap(),
        redirect_uris: serde_json::from_str(&row.try_get::<String, _>(2).unwrap())
            .unwrap_or_default(),
    }))
}

pub async fn create_code(state: &SharedState, code: &NewCode<'_>) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO oauth_codes (code_digest, client_id, user_id, organization_id, redirect_uri, code_challenge, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(digest(code.code))
    .bind(code.client_id)
    .bind(code.user_id)
    .bind(code.organization_id)
    .bind(code.redirect_uri)
    .bind(code.code_challenge)
    .bind(code.expires_at)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Marks the code used and returns it as it was before, so the caller can
/// tell a first exchange (`used == false`) from a replay.
pub async fn take_code(state: &SharedState, code: &str) -> Result<Option<CodeRow>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT code_digest, client_id, user_id, organization_id, redirect_uri, code_challenge, expires_at, used_at FROM oauth_codes WHERE code_digest = ?1",
    )
    .bind(digest(code))
    .fetch_optional(&conn)
    .await
    .map_err(|e| e.to_string())?;
    let Some(row) = row else { return Ok(None) };
    let code_digest: String = row.try_get(0).unwrap();
    // The `used_at IS NULL` guard makes the first exchange the only one that flips it.
    let first = sqlx::query(
        "UPDATE oauth_codes SET used_at = strftime('%Y-%m-%d %H:%M:%S', 'now') WHERE code_digest = ?1 AND used_at IS NULL",
    )
    .bind(&code_digest)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected()
        == 1;
    Ok(Some(CodeRow {
        code_digest,
        client_id: row.try_get(1).unwrap(),
        user_id: row.try_get(2).unwrap(),
        organization_id: row.try_get(3).unwrap(),
        redirect_uri: row.try_get(4).unwrap(),
        code_challenge: row.try_get(5).unwrap(),
        expires_at: row.try_get(6).unwrap(),
        used: !first,
    }))
}

pub async fn create_refresh(state: &SharedState, refresh: &NewRefresh<'_>) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO oauth_refresh_tokens (token_digest, client_id, user_id, organization_id, access_token_id, code_digest, expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .bind(digest(refresh.token))
    .bind(refresh.client_id)
    .bind(refresh.user_id)
    .bind(refresh.organization_id)
    .bind(refresh.access_token_id)
    .bind(refresh.code_digest)
    .bind(refresh.expires_at)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Revokes the refresh token and returns it, or `None` when it is unknown or
/// already revoked. Expiry is the caller's check.
pub async fn take_refresh(state: &SharedState, token: &str) -> Result<Option<RefreshRow>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "UPDATE oauth_refresh_tokens SET revoked_at = strftime('%Y-%m-%d %H:%M:%S', 'now') WHERE token_digest = ?1 AND revoked_at IS NULL RETURNING client_id, user_id, organization_id, access_token_id, code_digest, expires_at",
    )
    .bind(digest(token))
    .fetch_optional(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.map(|row| RefreshRow {
        client_id: row.try_get(0).unwrap(),
        user_id: row.try_get(1).unwrap(),
        organization_id: row.try_get(2).unwrap(),
        access_token_id: row.try_get(3).unwrap(),
        code_digest: row.try_get(4).unwrap(),
        expires_at: row.try_get(5).unwrap(),
    }))
}

/// Everything one approval produced, across rotations: its refresh tokens and
/// the access tokens behind them. Used when a code is replayed, and whenever a
/// grant is revoked. The session record is marked closed with them.
pub async fn revoke_by_code(state: &SharedState, code_digest: &str) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE personal_access_tokens SET revoked_at = strftime('%Y-%m-%d %H:%M:%S', 'now') WHERE revoked_at IS NULL AND id IN (SELECT access_token_id FROM oauth_refresh_tokens WHERE code_digest = ?1)",
    )
    .bind(code_digest)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?;
    sqlx::query(
        "UPDATE oauth_refresh_tokens SET revoked_at = strftime('%Y-%m-%d %H:%M:%S', 'now') WHERE code_digest = ?1 AND revoked_at IS NULL",
    )
    .bind(code_digest)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?;
    close_session(state, code_digest).await
}

/// One row of the account's session list for an MCP agent grant.
pub struct McpSessionRow {
    pub id: String,
    pub client_name: String,
    pub agent: String,
    pub created_at: String,
    pub expires_at: String,
    pub closed_at: Option<String>,
}

/// Records one session per OAuth grant, keyed by `code_digest`: created on the
/// first token exchange, and moved forward on every rotation so `expires_at`
/// shows the rolling expiry the grant really has.
pub async fn record_session(
    state: &SharedState,
    code_digest: &str,
    client_id: &str,
    user_id: &str,
    organization_id: &str,
    client_name: &str,
    agent: &str,
    expires_at: &str,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO mcp_oauth_sessions (id, code_digest, client_id, user_id, organization_id, client_name, agent, expires_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
         ON CONFLICT(code_digest) DO UPDATE SET expires_at = excluded.expires_at",
    )
    .bind(uuid::Uuid::now_v7().to_string())
    .bind(code_digest)
    .bind(client_id)
    .bind(user_id)
    .bind(organization_id)
    .bind(client_name)
    .bind(agent)
    .bind(expires_at)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Marks a grant's session record closed. Idempotent.
pub async fn close_session(state: &SharedState, code_digest: &str) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE mcp_oauth_sessions SET closed_at = strftime('%Y-%m-%d %H:%M:%S', 'now') WHERE code_digest = ?1 AND closed_at IS NULL",
    )
    .bind(code_digest)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Every session record for one user, newest first. Closed rows are kept: they
/// are the history of which agents had access and when.
pub async fn list_sessions(
    state: &SharedState,
    user_id: &str,
) -> Result<Vec<McpSessionRow>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT id, client_name, agent, created_at, expires_at, closed_at FROM mcp_oauth_sessions WHERE user_id = ?1 ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .map(|row| McpSessionRow {
            id: row.try_get(0).unwrap_or_default(),
            client_name: row.try_get(1).unwrap_or_default(),
            agent: row.try_get(2).unwrap_or_default(),
            created_at: row.try_get(3).unwrap_or_default(),
            expires_at: row.try_get(4).unwrap_or_default(),
            closed_at: row.try_get(5).unwrap_or(None),
        })
        .collect())
}

/// The grant behind one session record, when the id is the user's.
pub async fn take_grant(
    state: &SharedState,
    session_id: &str,
    user_id: &str,
) -> Result<Option<String>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT code_digest FROM mcp_oauth_sessions WHERE id = ?1 AND user_id = ?2",
    )
    .bind(session_id)
    .bind(user_id)
    .fetch_optional(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(row.map(|row| row.try_get(0).unwrap_or_default()))
}

/// Revokes every MCP grant the user has: the tokens behind each one and the
/// session record itself. Returns how many records were closed.
pub async fn revoke_all_grants_for_user(state: &SharedState, user_id: &str) -> Result<u64, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE personal_access_tokens SET revoked_at = strftime('%Y-%m-%d %H:%M:%S', 'now') WHERE revoked_at IS NULL AND id IN (SELECT access_token_id FROM oauth_refresh_tokens WHERE user_id = ?1)",
    )
    .bind(user_id)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?;
    sqlx::query(
        "UPDATE oauth_refresh_tokens SET revoked_at = strftime('%Y-%m-%d %H:%M:%S', 'now') WHERE user_id = ?1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?;
    let closed = sqlx::query(
        "UPDATE mcp_oauth_sessions SET closed_at = strftime('%Y-%m-%d %H:%M:%S', 'now') WHERE user_id = ?1 AND closed_at IS NULL",
    )
    .bind(user_id)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?
    .rows_affected();
    Ok(closed)
}
