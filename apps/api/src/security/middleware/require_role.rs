use axum::{extract::Request, http::StatusCode, middleware::Next, response::Response};
use futures_util::future::BoxFuture;

use crate::security::middleware::auth::AuthUser;

/// Organization-role checks live in `handler::common`, not here: they need a DB
/// lookup, and middleware has no connection. This module only carries the gates
/// that can be decided from the fields already cached on `AuthUser`.
///
/// Builds a router-layer middleware that rejects the request (403) unless
/// `predicate` accepts the caller's `AuthUser`. Must be layered so it runs
/// *after* `auth_middleware`, which is what populates `Extension<AuthUser>`
/// in the request extensions — no DB access happens here, so `predicate`
/// can only see fields already cached on `AuthUser` (e.g. `is_sysadmin`).
///
/// Generic on purpose so any role gate can reuse it, e.g.:
/// `require_role(|user| user.is_sysadmin)`.
pub fn require_role<F>(
    predicate: F,
) -> impl Fn(Request, Next) -> BoxFuture<'static, Result<Response, StatusCode>> + Clone
where
    F: Fn(&AuthUser) -> bool + Clone + Send + Sync + 'static,
{
    move |request: Request, next: Next| {
        let predicate = predicate.clone();
        Box::pin(async move {
            let allowed = request
                .extensions()
                .get::<AuthUser>()
                .is_some_and(|user| predicate(user));
            if !allowed {
                return Err(StatusCode::FORBIDDEN);
            }
            Ok(next.run(request).await)
        })
    }
}

/// Convenience gate for the global SysAdmin flag.
pub fn require_sysadmin()
-> impl Fn(Request, Next) -> BoxFuture<'static, Result<Response, StatusCode>> + Clone {
    require_role(|user| user.is_sysadmin)
}
