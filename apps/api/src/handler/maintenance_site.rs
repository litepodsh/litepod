//! The public route that serves an application's maintenance site.
//!
//! Caddy rewrites a visitor's request onto `/__litepod/maintenance/{application_id}/...`
//! and proxies it here while the application is in maintenance with an uploaded
//! site. There is no session and no token: what protects it is that nothing is
//! served unless the application is in maintenance, and only to a request whose
//! `Host` is one of the application's own domains (see `maintenance::serve_site`).
//!
//! It is mounted outside the API's rate limiter, next to the UI fallback: every
//! visitor of every application in maintenance reaches it through Caddy, from
//! the same address, so a per-IP limit would turn a busy page into a 429.

use std::collections::HashMap;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};

use crate::{deployment::maintenance, state::state::SharedState};

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "Not found").into_response()
}

pub async fn serve_site_handler(
    State(state): State<SharedState>,
    Path(params): Path<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let Some(application_id) = params.get("application_id") else {
        return not_found();
    };
    let path = params.get("path").map(String::as_str).unwrap_or("");
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let Some(file) = maintenance::serve_site(&state, application_id, host, path).await else {
        return not_found();
    };

    let status = StatusCode::from_u16(file.status).unwrap_or(StatusCode::SERVICE_UNAVAILABLE);
    let mut response = Response::new(axum::body::Body::from(file.body));
    *response.status_mut() = status;
    let out = response.headers_mut();
    out.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(file.content_type),
    );
    // A temporary page: nothing between here and the visitor may keep it.
    out.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    out.insert("x-robots-tag", HeaderValue::from_static("noindex"));
    out.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if status == StatusCode::SERVICE_UNAVAILABLE {
        out.insert(header::RETRY_AFTER, HeaderValue::from_static("300"));
    }
    response
}
