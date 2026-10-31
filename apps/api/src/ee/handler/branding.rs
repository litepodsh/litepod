//! Enterprise white-label: the name, logo and favicon a self-hosted instance
//! shows. Registered only when `!state.is_cloud` (see `router.rs`).

use axum::{Extension, Json, extract::State, response::IntoResponse};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use utoipa::ToSchema;

use crate::dtos::response::{AppError, ok};
use crate::security::middleware::auth::AuthUser;
use crate::state::state::SharedState;

const MAX_NAME_CHARS: usize = 40;
const MAX_URL_BYTES: usize = 2048;

/// `None` on a field means the Litepod default.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct Branding {
    pub name: Option<String>,
    pub logo_url: Option<String>,
    pub favicon_url: Option<String>,
}

async fn load(state: &SharedState) -> Result<Branding, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT name, logo_url, favicon_url FROM instance_branding WHERE id = 1")
        .fetch_one(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    Ok(Branding {
        name: row.get("name"),
        logo_url: row.get("logo_url"),
        favicon_url: row.get("favicon_url"),
    })
}

/// Cloud has no white-label, but every page still asks: answer with the
/// defaults instead of a 404 (which also skips CORS and shows up as a CORS error).
pub async fn default_handler() -> impl IntoResponse {
    ok(Branding {
        name: None,
        logo_url: None,
        favicon_url: None,
    })
}

/// Public: the login page renders it before there is a session.
pub async fn get_handler(State(state): State<SharedState>) -> Result<impl IntoResponse, AppError> {
    Ok(ok(load(&state).await?))
}

fn clean_name(value: Option<String>) -> Result<Option<String>, AppError> {
    let Some(value) = value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    else {
        return Ok(None);
    };
    if value.chars().count() > MAX_NAME_CHARS {
        return Err(AppError::ValidationError(format!(
            "Name must be at most {MAX_NAME_CHARS} characters"
        )));
    }
    Ok(Some(value))
}

/// An `http(s)://` URL or a path on this instance. `//host` is protocol-relative,
/// so it is refused along with every other scheme (`javascript:`, `data:`).
fn clean_url(value: Option<String>, field: &str) -> Result<Option<String>, AppError> {
    let Some(value) = value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    else {
        return Ok(None);
    };
    let local_path = value.starts_with('/') && !value.starts_with("//");
    if value.len() > MAX_URL_BYTES
        || !(value.starts_with("https://") || value.starts_with("http://") || local_path)
        || value.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(AppError::ValidationError(format!(
            "{field} must be an http(s) URL or a path starting with /"
        )));
    }
    Ok(Some(value))
}

pub async fn update_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    Json(request): Json<Branding>,
) -> Result<impl IntoResponse, AppError> {
    if !user.is_sysadmin {
        return Err(AppError::InsufficientPermissions);
    }
    let name = clean_name(request.name)?;
    let logo_url = clean_url(request.logo_url, "Logo")?;
    let favicon_url = clean_url(request.favicon_url, "Favicon")?;
    let conn = crate::git::repositories::db::get_conn(&state).await;
    sqlx::query(
        "UPDATE instance_branding SET name = ?1, logo_url = ?2, favicon_url = ?3, updated_at = datetime('now') WHERE id = 1",
    )
    .bind(&name)
    .bind(&logo_url)
    .bind(&favicon_url)
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let _ = crate::security::security_events::record(
        &state,
        None,
        &user,
        "settings.branding_updated",
        name.as_deref(),
    )
    .await;
    Ok(ok(Branding {
        name,
        logo_url,
        favicon_url,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_values_fall_back_to_the_default() {
        assert_eq!(clean_name(Some("  ".into())).unwrap(), None);
        assert_eq!(clean_url(None, "Logo").unwrap(), None);
    }

    #[test]
    fn name_is_trimmed_and_capped() {
        assert_eq!(
            clean_name(Some(" Acme ".into())).unwrap().as_deref(),
            Some("Acme")
        );
        assert!(clean_name(Some("x".repeat(MAX_NAME_CHARS + 1))).is_err());
    }

    #[test]
    fn urls_accept_http_and_local_paths_only() {
        for ok in [
            "https://a.test/logo.svg",
            "http://a.test/x.png",
            "/static/logo.svg",
        ] {
            assert!(clean_url(Some(ok.into()), "Logo").is_ok(), "{ok}");
        }
        for bad in [
            "javascript:alert(1)",
            "data:image/png;base64,AAAA",
            "//evil.test/x",
            "ftp://a/b",
            "https://a b",
        ] {
            assert!(clean_url(Some(bad.into()), "Logo").is_err(), "{bad}");
        }
    }
}
