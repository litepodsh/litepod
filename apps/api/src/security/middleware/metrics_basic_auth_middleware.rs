use axum::{
    http::{Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

pub async fn metrics_basic_auth_middleware(
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(auth) if auth.starts_with("Basic ") => {
            // Both are validated as set at startup in main.rs — no insecure default here.
            let username = std::env::var("METRICS_USERNAME")
                .expect("METRICS_USERNAME must be set (validated at startup in main.rs)");
            let password = std::env::var("METRICS_PASSWORD")
                .expect("METRICS_PASSWORD must be set (validated at startup in main.rs)");
            let credentials = format!("{}:{}", username, password);
            let encoded =
                base64::Engine::encode(&base64::engine::general_purpose::STANDARD, credentials);

            if auth == format!("Basic {}", encoded) {
                next.run(request).await
            } else {
                (
                    StatusCode::UNAUTHORIZED,
                    [(header::WWW_AUTHENTICATE, "Basic realm=\"metrics\"")],
                    "Unauthorized",
                )
                    .into_response()
            }
        }
        _ => (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Basic realm=\"metrics\"")],
            "Unauthorized",
        )
            .into_response(),
    }
}
