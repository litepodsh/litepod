use axum::{body::Body, extract::Request, middleware::Next, response::Response};
use sentry::protocol::SpanStatus;

pub async fn sentry_transaction_middleware(request: Request<Body>, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path();
    let route = request
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|matched| matched.as_str())
        .unwrap_or(path);

    let name = format!("{method} {route}");
    let ctx = sentry::TransactionContext::new(&name, "http.server");

    let transaction = sentry::start_transaction(ctx);
    sentry::configure_scope(|scope| {
        scope.set_span(Some(transaction.clone().into()));
    });

    let response = next.run(request).await;

    let status_code = response.status();
    let span_status = if status_code.is_server_error() {
        SpanStatus::InternalError
    } else if status_code.is_client_error() {
        SpanStatus::InvalidArgument
    } else {
        SpanStatus::Ok
    };
    transaction.set_status(span_status);
    transaction.set_data("http.status_code", status_code.as_u16().into());
    transaction.finish();

    response
}
