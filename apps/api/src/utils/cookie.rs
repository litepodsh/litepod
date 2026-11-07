use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use cookie::time::Duration as CookieDuration;

use crate::state::state::SharedState;

/// Name over plain http (`just dev`, http-only installs), where `Secure` is impossible.
pub const SESSION_COOKIE: &str = "session_token";

/// Name over https. The browser only accepts a `__Host-` cookie that is `Secure`,
/// `Path=/` and without `Domain`, so an app on a sibling subdomain cannot overwrite it.
pub const HOST_SESSION_COOKIE: &str = "__Host-session_token";

/// The session token from either cookie name, `__Host-` first.
/// shortcut: `SESSION_COOKIE` is still read on https so sessions opened before the
/// rename survive; stop reading it there one release after it ships.
pub fn session_token(jar: &CookieJar) -> Option<String> {
    jar.get(HOST_SESSION_COOKIE)
        .or_else(|| jar.get(SESSION_COOKIE))
        .map(|cookie| cookie.value().to_owned())
}

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
    let name = if secure { HOST_SESSION_COOKIE } else { SESSION_COOKIE };
    build_named(name, token, secure, max_age)
}

fn build_named(
    name: &'static str,
    token: &str,
    secure: bool,
    max_age: CookieDuration,
) -> Cookie<'static> {
    Cookie::build((name, token.to_owned()))
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
    let secure = secure(state).await;
    let jar = jar.add(build(&token, secure, CookieDuration::days(3)));
    if secure {
        // Drop a session cookie from before the `__Host-` rename.
        jar.add(build_named(SESSION_COOKIE, "", true, CookieDuration::seconds(0)))
    } else {
        jar
    }
}

/// Overwrite the cookie with an expired empty one, under both names. The
/// attributes have to match the ones it was set with, or some browsers keep the
/// original alongside it.
pub async fn clear_session_cookie(state: &SharedState, jar: CookieJar) -> CookieJar {
    let secure = secure(state).await;
    let jar = jar.add(build_named(SESSION_COOKIE, "", secure, CookieDuration::seconds(0)));
    if secure {
        jar.add(build_named(HOST_SESSION_COOKIE, "", true, CookieDuration::seconds(0)))
    } else {
        jar
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_uses_the_host_prefix_and_both_names_are_read() {
        let secure = build("t", true, CookieDuration::days(1));
        assert_eq!(secure.name(), HOST_SESSION_COOKIE);
        assert_eq!(secure.path(), Some("/"));
        assert_eq!(secure.domain(), None);
        assert_eq!(secure.secure(), Some(true));
        assert_eq!(build("t", false, CookieDuration::days(1)).name(), SESSION_COOKIE);

        let legacy = CookieJar::new().add(Cookie::new(SESSION_COOKIE, "old"));
        assert_eq!(session_token(&legacy).as_deref(), Some("old"));
        let both = legacy.add(Cookie::new(HOST_SESSION_COOKIE, "new"));
        assert_eq!(session_token(&both).as_deref(), Some("new"));
        assert_eq!(session_token(&CookieJar::new()), None);
    }
}
