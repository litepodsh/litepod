//! Shared HTTP client for calls to fixed third-party APIs (Turnstile, Resend, Polar, Loki).
//!
//! `reqwest::Client::new()` has no timeout, so a hung upstream kept the user's request
//! open forever, and each call paid a new TLS handshake. One client reuses connections.

use std::sync::LazyLock;
use std::time::Duration;

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        // Fixed API endpoints never need to redirect; following one would send the
        // credentials (secret, API key) somewhere else.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("HTTP client with static configuration")
});

pub fn client() -> &'static reqwest::Client {
    &CLIENT
}
