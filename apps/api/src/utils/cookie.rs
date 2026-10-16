use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use cookie::time::Duration as CookieDuration;

use crate::state::state::SharedState;

pub const SESSION_COOKIE: &str = "session_token";

/// Whether the session cookie should carry `Secure`.
///
/// `Secure` tells the browser never to send the cookie over plain HTTP. Without
/// it, any downgraded or mixed-content request puts a live session token on the
/// wire in the clear, where anyone on the path can take it — `grep '\.secure('`
/// over `api/src` used to return nothing at all.
///
/// It cannot simply be set unconditionally: a browser silently discards a
/// `Secure` cookie that arrives over http, so hard-coding it would leave
/// `just dev` on localhost, and any self-hosted install running plain HTTP,
/// unable to log anyone in — with no error to explain why.
///
/// So it follows the scheme the browser is actually using, which is what
/// `public_base_url` resolves (configured domain plus its https flag, falling
/// back to `state.public_url`).
async fn secure(state: &SharedState) -> bool {
    crate::handler::settings::public_base_url(state)
        .await
        .starts_with("https://")
}

fn build(token: &str, secure: bool, max_age: CookieDuration) -> Cookie<'static> {
    Cookie::build((SESSION_COOKIE, token.to_owned()))
        .http_only(true)
        .secure(secure)
        // `Lax` rather than `Strict` so following a link into the app keeps the
        // session; CSRF on mutations is covered separately by `middleware::csrf`.
        .same_site(SameSite::Lax)
        .path("/")
        .max_age(max_age)
        .build()
}

pub async fn add_session_cookie(state: &SharedState, jar: CookieJar, token: String) -> CookieJar {
    jar.add(build(&token, secure(state).await, CookieDuration::days(3)))
}

/// Overwrite the cookie with an expired empty one. The attributes have to match
/// the ones it was set with, or some browsers keep the original alongside it.
pub async fn clear_session_cookie(state: &SharedState, jar: CookieJar) -> CookieJar {
    jar.add(build("", secure(state).await, CookieDuration::seconds(0)))
}

/// Set at logout when the IdP session could not be ended from here (SAML,
/// IdPs without `end_session_endpoint`): the next SSO login from this browser
/// must re-enter credentials. Scoped to the auth API, never read by the UI.
pub const SSO_REAUTH_COOKIE: &str = "lp_sso_reauth";

fn build_reauth(value: &str, secure: bool, max_age: CookieDuration) -> Cookie<'static> {
    Cookie::build((SSO_REAUTH_COOKIE, value.to_owned()))
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Lax)
        .path("/api/v1/auth")
        .max_age(max_age)
        .build()
}

pub async fn add_sso_reauth_cookie(state: &SharedState, jar: CookieJar) -> CookieJar {
    jar.add(build_reauth(
        "1",
        secure(state).await,
        CookieDuration::days(30),
    ))
}

pub async fn clear_sso_reauth_cookie(state: &SharedState, jar: CookieJar) -> CookieJar {
    jar.add(build_reauth(
        "",
        secure(state).await,
        CookieDuration::seconds(0),
    ))
}
