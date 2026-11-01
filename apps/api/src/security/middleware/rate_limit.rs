use std::net::{IpAddr, SocketAddr};

use axum::{
    Json,
    body::Body,
    extract::{ConnectInfo, Request, State},
    http::{HeaderValue, StatusCode, header::HeaderName},
    middleware::Next,
    response::{IntoResponse, Response},
};
use chrono::Utc;
use governor::clock::{Clock, DefaultClock};

use crate::dtos::response::ApiResponse;
use crate::state::state::SharedState;
use crate::{scheduled_tasks::ephemeral::EphemeralStore, security::rate_limit::KeyedLimiter};

static RATELIMIT_LIMIT: HeaderName = HeaderName::from_static("x-ratelimit-limit");
static RATELIMIT_REMAINING: HeaderName = HeaderName::from_static("x-ratelimit-remaining");
static RETRY_AFTER: HeaderName = HeaderName::from_static("retry-after");

/// Real client IP behind Caddy's reverse proxy (which sets `X-Forwarded-For`), falling back
/// to the TCP peer address for requests that reach this process directly (e.g. local dev).
fn client_ip(request: &Request<Body>) -> IpAddr {
    let connect_info = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| *addr);
    client_ip_from_parts(request.headers(), connect_info)
}

/// Whether a peer is allowed to tell us who the real client is.
///
/// `X-Forwarded-For` is caller-supplied text. Trusting it unconditionally means
/// anyone can set a fresh value per request and walk straight past the login
/// failure ban and the captcha threshold, both of which are keyed on this
/// address — unlimited brute force from one machine.
///
/// The deployed topology is a reverse proxy on the same host or the same podman
/// network (`podman-compose.yml` puts Caddy in front). An application container
/// also sits on a private address on that network, so "any private peer" would
/// let a tenant forge the header — the proxy is therefore recognized by its
/// resolved address (`proxy_peer`), not by the address range it happens to be
/// in.
///
/// `TRUSTED_PROXY_IPS` overrides this with a comma-separated list of exact peer
/// addresses, for deployments whose proxy sits on an address that is not the
/// loopback, a link-local address, or the resolved Caddy address (a managed
/// load balancer, say).
fn peer_may_set_forwarded_for(peer: IpAddr) -> bool {
    if let Ok(list) = std::env::var("TRUSTED_PROXY_IPS") {
        let list = list.trim();
        if !list.is_empty() {
            return list
                .split(',')
                .filter_map(|entry| entry.trim().parse::<IpAddr>().ok())
                .any(|allowed| allowed == peer);
        }
    }
    trusted_proxy_peer(peer, proxy_peer())
}

/// The control-plane reverse proxy's address on the shared network, resolved on
/// demand and cached briefly. Caddy is the only host that may legitimately set
/// `X-Forwarded-For`; an application container shares the private range, so the
/// range itself is not trusted. The hostname is stable within a network
/// lifetime, and a short TTL re-resolves it after Caddy is recreated (a core
/// service update), when its address may change.
fn proxy_peer() -> Option<IpAddr> {
    use std::net::ToSocketAddrs;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CADDY: Mutex<Option<(Instant, Option<IpAddr>)>> = Mutex::new(None);
    const TTL: Duration = Duration::from_secs(60);

    let mut cache = CADDY.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((at, ip)) = *cache
        && at.elapsed() < TTL
    {
        return ip;
    }
    let ip = ("litepod-caddy", 80u16)
        .to_socket_addrs()
        .ok()
        .and_then(|addrs| {
            addrs
                .map(|addr| addr.ip())
                .find(|ip| matches!(ip, IpAddr::V4(v4) if !v4.is_loopback()))
        });
    *cache = Some((Instant::now(), ip));
    ip
}

/// Pure trust decision for a peer, given the resolved proxy address. Loopback
/// (a same-host proxy dialing a published port) and link-local (host-gateway)
/// peers are always the proxy, never a tenant.
fn trusted_proxy_peer(peer: IpAddr, proxy: Option<IpAddr>) -> bool {
    match peer {
        IpAddr::V4(ip) if ip.is_loopback() || ip.is_link_local() => return true,
        // IPv6 loopback (`::1`) and link-local (`fe80::/10`).
        IpAddr::V6(ip) if ip.is_loopback() || (ip.segments()[0] & 0xffc0) == 0xfe80 => {
            return true;
        }
        _ => {}
    }
    proxy == Some(peer)
}

/// The address to attribute this request to, for rate limiting and login bans.
///
/// Shared with handlers that only have `HeaderMap` + `ConnectInfo` extractors (not a raw
/// `Request`), e.g. the Turnstile siteverify call in `handler/auth.rs`.
pub(crate) fn client_ip_from_parts(
    headers: &axum::http::HeaderMap,
    connect_info: Option<SocketAddr>,
) -> IpAddr {
    let peer = connect_info
        .map(|addr| addr.ip())
        .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));

    if !peer_may_set_forwarded_for(peer) {
        return peer;
    }

    // Left-most entry is the original client as recorded by the first proxy.
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .and_then(|first| first.trim().parse::<IpAddr>().ok())
        .unwrap_or(peer)
}

fn too_many_requests(message: &str) -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(ApiResponse::<()> {
            error: Some(message.to_string()),
            data: None,
        }),
    )
        .into_response()
}

/// Runs `request` through `limiter` keyed on `ip`, setting `X-RateLimit-Limit` /
/// `X-RateLimit-Remaining` on the eventual response either way, and `Retry-After` when the
/// limit is exceeded — so clients (and curious sysadmins poking at it with curl) can see the
/// configured quota and how much of it is left, not just a bare 429.
async fn apply_limiter(
    limiter: &KeyedLimiter,
    ephemeral: &EphemeralStore,
    ip: IpAddr,
    per_second: u32,
    burst: u32,
    scope: &str,
    message: &str,
    request: Request<Body>,
    next: Next,
) -> Response {
    let key = format!("litepod:rate-limit:{scope}:{ip}");
    match ephemeral.token_bucket(&key, per_second, burst).await {
        Ok(snapshot) if snapshot.allowed => {
            let mut response = next.run(request).await;
            let headers = response.headers_mut();
            headers.insert(RATELIMIT_LIMIT.clone(), HeaderValue::from(burst));
            headers.insert(
                RATELIMIT_REMAINING.clone(),
                HeaderValue::from(snapshot.remaining),
            );
            return response;
        }
        Ok(snapshot) => {
            let mut response = too_many_requests(message);
            let headers = response.headers_mut();
            headers.insert(RATELIMIT_LIMIT.clone(), HeaderValue::from(burst));
            headers.insert(RATELIMIT_REMAINING.clone(), HeaderValue::from(0u32));
            headers.insert(
                RETRY_AFTER.clone(),
                HeaderValue::from(snapshot.retry_after_secs.max(1)),
            );
            return response;
        }
        Err(error) => {
            tracing::debug!(%error, scope, "Dragonfly rate limit unavailable; using local limiter")
        }
    }
    match limiter.check_key(&ip) {
        Ok(snapshot) => {
            let limit = snapshot.quota().burst_size().get();
            let remaining = snapshot.remaining_burst_capacity();
            let mut response = next.run(request).await;
            let headers = response.headers_mut();
            headers.insert(RATELIMIT_LIMIT.clone(), HeaderValue::from(limit));
            headers.insert(RATELIMIT_REMAINING.clone(), HeaderValue::from(remaining));
            response
        }
        Err(not_until) => {
            let retry_after = not_until.wait_time_from(DefaultClock::default().now());
            let mut response = too_many_requests(message);
            let headers = response.headers_mut();
            headers.insert(
                RATELIMIT_LIMIT.clone(),
                HeaderValue::from(not_until.quota().burst_size().get()),
            );
            headers.insert(RATELIMIT_REMAINING.clone(), HeaderValue::from(0u32));
            headers.insert(
                RETRY_AFTER.clone(),
                HeaderValue::from(retry_after.as_secs().max(1)),
            );
            response
        }
    }
}

pub async fn global_rate_limit_middleware(
    State(state): State<SharedState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let ip = client_ip(&request);
    let limiter = {
        let guard = state.rate_limiter.read().await;
        if !guard.config.enabled {
            return next.run(request).await;
        }
        (
            guard.global.clone(),
            guard.config.global_per_second,
            guard.config.global_burst,
        )
    };
    apply_limiter(
        &limiter.0,
        &state.ephemeral,
        ip,
        limiter.1,
        limiter.2,
        "global",
        "Too many requests",
        request,
        next,
    )
    .await
}

pub async fn auth_rate_limit_middleware(
    State(state): State<SharedState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let ip = client_ip(&request);
    let limiter = {
        let guard = state.rate_limiter.read().await;
        if !guard.config.enabled {
            return next.run(request).await;
        }
        (
            guard.auth.clone(),
            guard.config.auth_per_second,
            guard.config.auth_burst,
        )
    };
    apply_limiter(
        &limiter.0,
        &state.ephemeral,
        ip,
        limiter.1,
        limiter.2,
        "auth",
        "Too many requests to authentication endpoints",
        request,
        next,
    )
    .await
}

/// Tracks failed logins per IP and bans the IP for a configured duration once it crosses
/// the failed-attempt threshold. Separate from `auth_rate_limit_middleware`: a request-rate
/// limiter alone doesn't stop a slow brute-force that stays under the rate but keeps failing.
pub async fn login_ban_middleware(
    State(state): State<SharedState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let ip = client_ip(&request);
    if !state.rate_limiter.read().await.config.enabled {
        return next.run(request).await;
    }
    let remote_key = format!("litepod:login:{ip}");
    let remote_available = match state
        .ephemeral
        .remaining_ttl(&format!("{remote_key}:ban"))
        .await
    {
        Ok(Some(retry_after)) => {
            let mut response = too_many_requests("Too many failed login attempts, try again later");
            response
                .headers_mut()
                .insert(RETRY_AFTER.clone(), HeaderValue::from(retry_after));
            return response;
        }
        Ok(None) => true,
        Err(error) => {
            tracing::debug!(%error, "Dragonfly login ban unavailable; using local ban");
            false
        }
    };

    if !remote_available {
        let guard = state.rate_limiter.read().await;
        if guard.config.enabled {
            if let Some(banned_until) = guard
                .login_attempts
                .get(&ip)
                .and_then(|entry| entry.banned_until)
                .filter(|banned_until| *banned_until > Utc::now())
            {
                let mut response =
                    too_many_requests("Too many failed login attempts, try again later");
                let retry_after = (banned_until - Utc::now()).num_seconds().max(1) as u64;
                response
                    .headers_mut()
                    .insert(RETRY_AFTER.clone(), HeaderValue::from(retry_after));
                return response;
            }
        }
    }

    let response = next.run(request).await;

    if remote_available {
        let config = state.rate_limiter.read().await.config;
        if !config.enabled {
            return response;
        }
        let result: Result<(), String> = if response.status() == StatusCode::OK {
            match state
                .ephemeral
                .delete(&format!("{remote_key}:failures"))
                .await
            {
                Ok(()) => state.ephemeral.delete(&format!("{remote_key}:ban")).await,
                Err(error) => Err(error),
            }
        } else if response.status() == StatusCode::UNAUTHORIZED {
            state
                .ephemeral
                .record_login_failure(
                    &remote_key,
                    config
                        .auth_max_attempts
                        .saturating_mul(IP_BAN_ATTEMPT_MULTIPLIER),
                    (config.auth_ban_minutes.max(1) as u64) * 60,
                )
                .await
                .map(|_| ())
        } else {
            Ok(())
        };
        if result.is_ok() {
            return response;
        }
        tracing::debug!("Dragonfly login ban update failed; recording locally");
    }

    let mut guard = state.rate_limiter.write().await;
    if !guard.config.enabled {
        return response;
    }
    let max_attempts = guard
        .config
        .auth_max_attempts
        .saturating_mul(IP_BAN_ATTEMPT_MULTIPLIER);
    let ban_minutes = guard.config.auth_ban_minutes;
    let entry = guard.login_attempts.entry(ip).or_default();
    if response.status() == StatusCode::OK {
        entry.failed_count = 0;
        entry.banned_until = None;
    } else if response.status() == StatusCode::UNAUTHORIZED {
        entry.failed_count += 1;
        if entry.failed_count >= max_attempts {
            entry.banned_until = Some(Utc::now() + chrono::Duration::minutes(ban_minutes));
        }
    }
    response
}

/// The per-IP ban is a backstop, not the primary control: the per-account ban
/// (`account_login_*`) locks a brute-forced account across every IP. So the IP
/// ban only trips on mass flooding — this many times the per-account threshold
/// of total failures from one address — which keeps a shared office/NAT IP from
/// being blocked by normal, scattered mistyped passwords.
const IP_BAN_ATTEMPT_MULTIPLIER: u32 = 10;

/// Per-account login key, digested so a raw email never becomes a store key.
/// Normalized (trim + lowercase) so the counter follows the account, not the
/// exact casing typed.
pub(crate) fn account_login_key(email: &str) -> String {
    let digest = crate::utils::security::digest_token(&email.trim().to_ascii_lowercase());
    format!("litepod:login-account:{digest}")
}

/// Seconds left on an account's login ban, or `None` when it is not banned — or
/// the store is unreachable, in which case this **fails open**: the per-IP
/// layers still apply, and refusing every login during a Dragonfly blip would be
/// worse than briefly losing the per-account lock.
pub(crate) async fn account_login_ban_remaining(state: &SharedState, email: &str) -> Option<u64> {
    state
        .ephemeral
        .remaining_ttl(&format!("{}:ban", account_login_key(email)))
        .await
        .ok()
        .flatten()
}

/// Records one failed attempt against the account and bans it past the
/// configured threshold. Best-effort; honours the global rate-limit toggle.
pub(crate) async fn record_account_login_failure(state: &SharedState, email: &str) {
    let config = state.rate_limiter.read().await.config;
    if !config.enabled {
        return;
    }
    let _ = state
        .ephemeral
        .record_login_failure(
            &account_login_key(email),
            config.auth_max_attempts,
            (config.auth_ban_minutes.max(1) as u64) * 60,
        )
        .await;
}

/// Clears an account's failure counter and ban. Called on a correct password so
/// the next person to use a shared machine starts clean.
pub(crate) async fn clear_account_login(state: &SharedState, email: &str) {
    let key = account_login_key(email);
    let _ = state.ephemeral.delete(&format!("{key}:failures")).await;
    let _ = state.ephemeral.delete(&format!("{key}:ban")).await;
}

/// Failed logins at or above this count (same IP, tracked by `login_ban_middleware`) require
/// a Turnstile token on the next attempt, well before the ban threshold
/// (`auth_max_attempts`, default 5) kicks in. Deliberately not admin-configurable like the
/// ban settings — this is a soft speed bump, not the enforcement layer.
pub(crate) const LOGIN_CAPTCHA_THRESHOLD: u32 = 2;

/// Current failed-login count for `ip`, read the same way `login_ban_middleware` writes it
/// (Dragonfly-backed counter first, in-memory map as fallback) so `login_handler` can decide
/// whether this attempt needs a Turnstile token before calling `auth::login`.
pub(crate) async fn login_failed_attempts(state: &SharedState, ip: IpAddr) -> u32 {
    let remote_key = format!("litepod:login:{ip}:failures");
    match state.ephemeral.get_counter(&remote_key).await {
        Ok(Some(count)) => return count,
        // Remote reachable and confirms zero failures — authoritative, don't fall through to
        // a local map that may be stale from an earlier Dragonfly outage.
        Ok(None) => return 0,
        Err(error) => tracing::debug!(%error, "Dragonfly failure count unavailable; using local"),
    }
    state
        .rate_limiter
        .read()
        .await
        .login_attempts
        .get(&ip)
        .map(|entry| entry.failed_count)
        .unwrap_or(0)
}

#[cfg(test)]
mod account_key_tests {
    use super::account_login_key;

    #[test]
    fn account_key_is_normalized_and_does_not_leak_the_email() {
        // Same account regardless of casing or surrounding whitespace.
        assert_eq!(
            account_login_key("  User@Example.com "),
            account_login_key("user@example.com")
        );
        // Different accounts get different keys.
        assert_ne!(
            account_login_key("a@example.com"),
            account_login_key("b@example.com")
        );
        // The raw address never appears in the store key (it is digested).
        let key = account_login_key("victim@example.com");
        assert!(key.starts_with("litepod:login-account:"));
        assert!(!key.contains("victim@example.com"), "email leaked into key: {key}");
    }
}

#[cfg(test)]
mod forwarded_for_tests {
    use super::*;
    use axum::http::HeaderMap;

    fn headers_with(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", value.parse().unwrap());
        headers
    }

    fn peer(addr: &str) -> Option<SocketAddr> {
        Some(SocketAddr::new(addr.parse().unwrap(), 12345))
    }

    #[test]
    fn a_public_peer_cannot_claim_to_be_someone_else() {
        // The A6 attack: spoof a fresh client address per request to sidestep the
        // login ban and the captcha threshold.
        let ip = client_ip_from_parts(&headers_with("1.2.3.4"), peer("203.0.113.9"));
        assert_eq!(ip.to_string(), "203.0.113.9");
    }

    #[test]
    fn a_private_peer_is_not_believed_without_the_proxy() {
        // An application container sits on the same private range as Caddy, so a
        // private peer is no longer trusted unless it matches the resolved proxy
        // address (unresolvable here, so the spoofed header is ignored).
        let ip = client_ip_from_parts(&headers_with("1.2.3.4"), peer("10.89.0.2"));
        assert_eq!(ip.to_string(), "10.89.0.2");
    }

    #[test]
    fn loopback_is_believed_and_the_left_most_entry_wins() {
        let ip = client_ip_from_parts(&headers_with("1.2.3.4, 10.0.0.1"), peer("127.0.0.1"));
        assert_eq!(ip.to_string(), "1.2.3.4");
    }

    #[test]
    fn a_trusted_peer_sending_junk_falls_back_to_the_peer() {
        let ip = client_ip_from_parts(&headers_with("not-an-ip"), peer("127.0.0.1"));
        assert_eq!(ip.to_string(), "127.0.0.1");
    }

    #[test]
    fn no_header_means_the_peer() {
        let ip = client_ip_from_parts(&HeaderMap::new(), peer("198.51.100.7"));
        assert_eq!(ip.to_string(), "198.51.100.7");
    }

    #[test]
    fn trusted_peer_is_loopback_link_local_or_the_resolved_proxy() {
        // Loopback and link-local peers are always the proxy.
        assert!(trusted_proxy_peer("127.0.0.1".parse().unwrap(), None));
        assert!(trusted_proxy_peer("169.254.0.1".parse().unwrap(), None));
        assert!(trusted_proxy_peer("::1".parse().unwrap(), None));
        // A private or public peer is only trusted when it is the proxy itself.
        assert!(!trusted_proxy_peer("10.89.0.2".parse().unwrap(), None));
        assert!(!trusted_proxy_peer("203.0.113.9".parse().unwrap(), None));
        assert!(
            trusted_proxy_peer("10.89.0.2".parse().unwrap(), Some("10.89.0.2".parse().unwrap()))
        );
        // IPv6 unique-local (fc00::/7) is a tenant range, not the proxy.
        assert!(!trusted_proxy_peer("fc00::1".parse().unwrap(), None));
    }
}
