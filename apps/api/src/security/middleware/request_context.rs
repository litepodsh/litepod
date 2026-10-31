//! Request provenance — IP, user-agent, request id — stamped once per request
//! onto `RequestContext` in extensions. `audit_log::record` reads from it so
//! callers don't have to thread the same six fields through every endpoint,
//! and a CLI script or background task that runs `record` directly can pass an
//! empty one without lying about what it doesn't have.

use std::net::{IpAddr, SocketAddr};

use axum::{
    body::Body,
    extract::{ConnectInfo, Request},
    http::{HeaderName, HeaderValue, header},
    middleware::Next,
    response::Response,
};
use uuid::Uuid;

use crate::security::middleware::rate_limit::client_ip_from_parts;

static REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

/// Snapshot of the inbound request, in the shape audit rows want to carry.
/// Fields are optional because background tasks and CLI seeders construct one
/// of these directly; pretending to know an IP we don't have would be
/// worse than saying nothing.
#[derive(Clone, Debug, Default)]
pub struct RequestContext {
    pub source_ip: Option<IpAddr>,
    pub user_agent: Option<String>,
    pub request_id: String,
}

impl RequestContext {
    /// Background / CLI: a row that records "I don't know who called me" on
    /// every field except the request id we generated ourselves.
    pub fn synthetic() -> Self {
        Self {
            source_ip: None,
            user_agent: None,
            request_id: Uuid::now_v7().to_string(),
        }
    }
}

tokio::task_local! {
    static CURRENT: RequestContext;
}

/// This request's context, for code that has no `Extension` to read it from
/// (`security_events::record` has ~40 callers). `None` in background tasks.
pub fn current() -> Option<RequestContext> {
    CURRENT.try_with(Clone::clone).ok()
}

/// Builds a `RequestContext` for `request` without taking anything out of it
/// (`headers` and the `ConnectInfo` extension are both read-only). Lives next
/// to the middleware so tests that exercise just the extraction can do so
/// without spinning up an axum router.
pub fn request_context_for(request: &Request<Body>) -> RequestContext {
    let connect_info = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| *addr);
    let source_ip = client_ip_from_parts(request.headers(), connect_info);
    let user_agent = request
        .headers()
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());
    let request_id = request
        .headers()
        .get(&REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .unwrap_or_else(|| Uuid::now_v7().to_string());
    RequestContext {
        source_ip: Some(source_ip),
        user_agent,
        request_id,
    }
}

/// Runs once per request. Pulls the bits of provenance we want to keep
/// (client IP, user agent, a generated request id reused or echoed via
/// `X-Request-Id`), stuffs them in `RequestContext` for downstream handlers,
/// and copies the id onto the response so the client can quote it back to
/// support when something goes wrong. Echoes a caller-supplied `X-Request-Id`
/// when present so external systems can correlate.
pub async fn request_context_middleware(mut request: Request<Body>, next: Next) -> Response {
    let context = request_context_for(&request);
    request.extensions_mut().insert(context.clone());
    let mut response = CURRENT.scope(context.clone(), next.run(request)).await;
    if let Ok(value) = HeaderValue::from_str(&context.request_id) {
        response
            .headers_mut()
            .insert(REQUEST_ID_HEADER.clone(), value);
    }
    response
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;

    use super::*;

    fn peer(addr: &str) -> ConnectInfo<SocketAddr> {
        ConnectInfo(SocketAddr::new(addr.parse().unwrap(), 12345))
    }

    fn with_peer(mut request: Request<Body>, addr: &str) -> Request<Body> {
        request.extensions_mut().insert(peer(addr));
        request
    }

    #[test]
    fn captures_ip_user_agent_and_generates_a_request_id() {
        let request = with_peer(
            Request::builder()
                .header("user-agent", "curl/8.0")
                .body(Body::empty())
                .unwrap(),
            "127.0.0.1",
        );
        let context = request_context_for(&request);
        assert_eq!(context.source_ip.unwrap().to_string(), "127.0.0.1");
        assert_eq!(context.user_agent.as_deref(), Some("curl/8.0"));
        assert!(!context.request_id.is_empty());
    }

    #[test]
    fn echoes_a_caller_supplied_request_id_when_provided() {
        let request = with_peer(
            Request::builder()
                .header("x-request-id", "client-correlation-42")
                .body(Body::empty())
                .unwrap(),
            "127.0.0.1",
        );
        let context = request_context_for(&request);
        assert_eq!(context.request_id, "client-correlation-42");
    }

    #[test]
    fn synthetic_context_has_only_an_id() {
        let context = RequestContext::synthetic();
        assert!(context.source_ip.is_none());
        assert!(context.user_agent.is_none());
        assert!(!context.request_id.is_empty());
    }
}
