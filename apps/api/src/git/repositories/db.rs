use std::time::Duration;

use crate::{dtos::response::AppError, state::state::SharedState};

const MAX_RETRYABLE_CONFLICT_ATTEMPTS: u32 = 5;

/// Returns a pool handle for issuing queries. `SqlitePool` is a cheap, `Arc`-backed
/// clone — callers don't check out a connection explicitly the way the old turso
/// `Connection` model required; sqlx checks one out per query internally. Kept as a
/// thin wrapper (rather than inlining `&state.db` at every call site) so existing
/// call sites' `let conn = get_conn(state).await;` line keeps compiling.
pub async fn get_conn(state: &SharedState) -> sqlx::SqlitePool {
    state.db.clone()
}

/// True when `error` is SQLite's "database is locked" (SQLITE_BUSY/SQLITE_LOCKED):
/// another connection briefly holds the write lock. Nothing wrong with the
/// statement itself — retry the whole transaction.
pub fn is_locked_error(error: &AppError) -> bool {
    matches!(error, AppError::DatabaseError(message) if message.to_ascii_lowercase().contains("database is locked"))
}

/// Reruns `operation` (a full transaction, opened fresh each call) when it fails
/// with a locked-database busy error, with a short linear backoff. Any other
/// error, or exhausting the attempt budget, is returned as-is.
pub async fn retry_on_stale_snapshot<T, F, Fut>(mut operation: F) -> Result<T, AppError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, AppError>>,
{
    let mut attempt = 0_u32;
    loop {
        match operation().await {
            Err(error)
                if attempt + 1 < MAX_RETRYABLE_CONFLICT_ATTEMPTS && is_locked_error(&error) =>
            {
                attempt += 1;
                tracing::warn!(attempt, %error, "retrying transaction after a database conflict");
                tokio::time::sleep(Duration::from_millis(20 * attempt as u64)).await;
            }
            other => return other,
        }
    }
}
