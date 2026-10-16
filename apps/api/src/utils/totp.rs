use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use totp_rs::{Algorithm, Secret, TOTP};

use crate::state::state::SharedState;
use crate::utils::security;

/// Steps of clock skew tolerated on either side of now. One 30-second step each
/// way is the usual allowance; the old code accepted `check(0)`, `check(1)` and
/// `check(2)`, which is skew in the *future* only and stretched the window to 90
/// seconds — three times the exposure, and asymmetric for no reason (a phone
/// running slow was rejected while one running 80 seconds fast was accepted).
const SKEW_STEPS: u64 = 1;
const STEP_SECS: u64 = 30;

/// How long a spent code stays remembered. Must outlast the acceptance window,
/// or a code could be replayed just after the record expires but while it is
/// still valid.
const REPLAY_TTL_SECS: u64 = (2 * SKEW_STEPS + 2) * STEP_SECS;

fn build(secret: &str) -> Option<TOTP> {
    let secret = Secret::Encoded(secret.to_string());
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        STEP_SECS,
        secret.to_bytes().ok()?,
        None,
        String::new(),
    )
    .ok()
}

/// Generate a new base32-encoded TOTP secret.
pub fn generate_secret() -> String {
    let secret = Secret::generate_secret();
    secret.to_encoded().to_string()
}

/// Build an otpauth:// URI for QR code rendering.
pub fn generate_otpauth_url(secret: &str, email: &str, issuer: &str) -> String {
    let secret = Secret::Encoded(secret.to_string());
    let totp = TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        STEP_SECS,
        secret.to_bytes().unwrap(),
        Some(issuer.to_string()),
        email.to_string(),
    )
    .ok();
    match totp {
        Some(t) => t.get_url(),
        None => format!(
            "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm=SHA1&digits=6&period=30",
            issuer, email, secret, issuer
        ),
    }
}

/// Verify a code against the current step and one step either side.
///
/// Shape-only: says nothing about whether the code was already used. Prefer
/// [`verify_code_once`] anywhere an attacker could replay a code they observed.
pub fn verify_code(secret: &str, code: &str) -> bool {
    let Some(totp) = build(secret) else {
        return false;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let current = now / STEP_SECS;
    (current.saturating_sub(SKEW_STEPS)..=current + SKEW_STEPS)
        .any(|step| totp.generate(step * STEP_SECS) == code)
}

/// Verify a TOTP code strictly (current window only).
/// Used during initial setup to confirm the user scanned the QR correctly.
pub fn verify_code_strict(secret: &str, code: &str) -> bool {
    build(secret).is_some_and(|totp| totp.check_current(code).unwrap_or(false))
}

fn spent_key(user_id: &str, code: &str) -> String {
    format!("totp:v1:spent:{user_id}:{}", security::digest_token(code))
}

/// In-process replay claim used only when the shared ephemeral store is
/// unreachable. Failing closed would lock out every 2FA user during a
/// Dragonfly outage, so this narrows the gap instead: the same process can
/// no longer be replayed, even though a second process (another replica)
/// still can't be seen from here.
///
/// ponytail: single-process only, no cross-replica coverage. Upgrade path:
/// none needed while the ephemeral store recovers on its own; if multi-replica
/// deployments become real, share this via a broadcast or a quorum of
/// processes instead of a local map.
static LOCAL_REPLAY: LazyLock<Mutex<HashMap<String, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn local_claim(key: &str, ttl: Duration) -> bool {
    let now = Instant::now();
    let mut guard = LOCAL_REPLAY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.retain(|_, expires_at| *expires_at > now);
    if guard.contains_key(key) {
        false
    } else {
        guard.insert(key.to_string(), now + ttl);
        true
    }
}

/// Verify a code and burn it, so the same six digits cannot be presented twice.
///
/// A TOTP code stays valid for the length of its window, which is long enough
/// for anyone who can observe it — shoulder-surfing, a phishing page relaying in
/// real time, a logged request — to reuse it. `set_json_if_absent` is the atomic
/// claim: the first caller records the code and proceeds, any later caller finds
/// it present and is refused.
///
/// Failing closed on an ephemeral-store outage would lock every 2FA user out, so
/// a store error falls back to a local, in-process replay claim instead of
/// plain verification, and is logged. See [`local_claim`].
pub async fn verify_code_once(
    state: &SharedState,
    user_id: &str,
    secret: &str,
    code: &str,
) -> bool {
    if !verify_code(secret, code) {
        return false;
    }
    match state
        .ephemeral
        .set_json_if_absent(&spent_key(user_id, code), &true, REPLAY_TTL_SECS)
        .await
    {
        Ok(claimed) => claimed,
        Err(error) => {
            let claimed = local_claim(
                &spent_key(user_id, code),
                Duration::from_secs(REPLAY_TTL_SECS),
            );
            tracing::warn!(
                %error,
                claimed,
                "TOTP replay store unavailable; using local replay guard"
            );
            claimed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_for(secret: &str, offset_steps: i64) -> String {
        let totp = build(secret).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        totp.generate(((now / STEP_SECS as i64 + offset_steps) * STEP_SECS as i64) as u64)
    }

    #[test]
    fn accepts_the_current_step_and_one_either_side() {
        let secret = generate_secret();
        for offset in [-1, 0, 1] {
            assert!(
                verify_code(&secret, &code_for(&secret, offset)),
                "offset {offset} should be accepted"
            );
        }
    }

    #[test]
    fn rejects_beyond_the_skew_window() {
        let secret = generate_secret();
        // The old implementation accepted +2 — a 90-second forward window.
        for offset in [-2, 2, 3] {
            assert!(
                !verify_code(&secret, &code_for(&secret, offset)),
                "offset {offset} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_a_code_from_a_different_secret() {
        let secret = generate_secret();
        let other = generate_secret();
        assert!(!verify_code(&secret, &code_for(&other, 0)));
    }

    #[test]
    fn local_claim_rejects_replay_within_ttl_but_not_a_different_key() {
        let key = format!(
            "test:local_claim:{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        assert!(
            local_claim(&key, Duration::from_secs(5)),
            "first claim should succeed"
        );
        assert!(
            !local_claim(&key, Duration::from_secs(5)),
            "replay should be rejected"
        );
        assert!(
            local_claim(&format!("{key}:other"), Duration::from_secs(5)),
            "a different key is unaffected"
        );
    }
}
