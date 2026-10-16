use chrono::{Duration, Utc};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    dtos::response::AppError,
    git::repositories::{organization, organization_member, user},
    state::state::SharedState,
    utils::security,
};

const TTL_SECS: u64 = 7_200;
const COOLDOWN_SECS: i64 = 60;
const MAX_DELIVERIES: u8 = 4;
const MAX_CODE_ATTEMPTS: u8 = 5;
const PER_EMAIL_MAX: u64 = 10;
const PER_EMAIL_WINDOW: u64 = 3_600;
const CODE_LEN: usize = 6;
const DIGIT_COUNT: usize = 4;

#[derive(Clone, Serialize, Deserialize)]
struct VerificationState {
    user_id: String,
    code_digest: String,
    code_attempts: u8,
    accepted_deliveries: u8,
    resend_available_at: i64,
    kind: VerificationKind,
}

#[derive(Clone, Serialize, Deserialize)]
enum VerificationKind {
    Registration,
    Reactivation {
        hashed_password: String,
        name: String,
    },
}

#[derive(Serialize)]
pub struct PendingResult {
    pub email: String,
    pub accepted_deliveries: u8,
    pub resend_available_at: i64,
    pub delivery_failed: bool,
    pub reactivating: bool,
}
#[derive(Serialize)]
pub struct ConfirmResult {
    pub verified: bool,
    pub reactivated: bool,
}

fn user_key(id: &str) -> String {
    format!("email-verification:v1:user:{id}")
}
fn code_key(digest: &str) -> String {
    format!("email-verification:v1:code:{digest}")
}
fn lock_key(id: &str) -> String {
    format!("email-verification:v1:lock:{id}")
}
fn attempts_key(email: &str) -> String {
    format!("email-verification:v1:attempts:{}", email.to_lowercase())
}
fn digest(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn now() -> chrono::DateTime<Utc> {
    Utc::now()
}
fn deadline() -> String {
    (now() + Duration::seconds(TTL_SECS as i64))
        .naive_utc()
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

pub fn generate_code() -> String {
    let mut rng = rand::rng();
    let mut indices: Vec<usize> = (0..CODE_LEN).collect();
    for i in 0..DIGIT_COUNT {
        let j = i + rng.random_range(0..CODE_LEN - i);
        indices.swap(i, j);
    }
    let digit_positions: std::collections::HashSet<usize> =
        indices.into_iter().take(DIGIT_COUNT).collect();
    let mut chars: Vec<char> = Vec::with_capacity(CODE_LEN);
    for i in 0..CODE_LEN {
        if digit_positions.contains(&i) {
            chars.push(char::from(b'0' + rng.random_range(0u8..10u8)));
        } else {
            chars.push(char::from(b'A' + rng.random_range(0u8..26u8)));
        }
    }
    chars.into_iter().collect()
}

fn normalize_code(raw: &str) -> Result<String, AppError> {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if cleaned.len() != CODE_LEN || !cleaned.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(AppError::ValidationError(
            "Code must be 6 characters (letters and numbers)".into(),
        ));
    }
    Ok(cleaned)
}

/// Delivers one transactional email through Resend. Shared by every flow that
/// mails a user (verification codes, the already-registered notice, password
/// reset), so they all fail the same way when `RESEND_API_KEY` is unset.
pub(crate) async fn deliver(email: &str, subject: &str, html: String) -> Result<(), ()> {
    let key = std::env::var("RESEND_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty())
        .ok_or(())?;
    let response = reqwest::Client::new()
        .post("https://api.resend.com/emails")
        .bearer_auth(key)
        .json(&serde_json::json!({
            "from": "No Reply <noreply@litepod.sh>",
            "to": [email],
            "subject": subject,
            "html": html,
        }))
        .send()
        .await
        .map_err(|_| ())?;
    response.error_for_status().map_err(|_| ())?;
    Ok(())
}

async fn send(state: &SharedState, email: &str, code: &str) -> Result<(), ()> {
    let _ = state;
    let html = format!(
        "<pre style=\"font-family: monospace; font-size: 32px; letter-spacing: 8px; text-align: center;\">  {}</pre>\
         <p>Enter this code on the verification page. It expires in 2 hours.</p>",
        code.chars()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    );
    deliver(email, "Your litepod verification code", html).await
}

/// Sent instead of an `AlreadyExists` error when someone tries to register an
/// address that already has an active account. The signup response is identical
/// either way, so this note is the only signal — and it goes to the address
/// itself, which is the one party entitled to know.
async fn send_already_registered_notice(email: &str) -> Result<(), ()> {
    let html = "<p>Someone just tried to create a litepod account with this email address.</p>\
         <p>You already have one, so nothing changed. If it was you, sign in instead — \
         and use the password reset link on the sign-in page if you've forgotten your password.</p>\
         <p>If it wasn't you, you can safely ignore this message.</p>"
        .to_string();
    deliver(email, "Someone tried to sign up with your email", html).await
}

async fn issue(state: &SharedState, state_value: &VerificationState) -> Result<String, AppError> {
    let code = generate_code();
    let code_digest = digest(&code);
    let record = VerificationState {
        code_digest: code_digest.clone(),
        code_attempts: 0,
        ..state_value.clone()
    };
    state
        .ephemeral
        .set_json(&code_key(&code_digest), &record.user_id, TTL_SECS)
        .await
        .map_err(AppError::InternalError)?;
    state
        .ephemeral
        .set_json(&user_key(&record.user_id), &record, TTL_SECS)
        .await
        .map_err(AppError::InternalError)?;
    Ok(code)
}

pub async fn register(
    state: &SharedState,
    email: &str,
    password: &str,
    name: &str,
    acceptance: &crate::users::legal::Acceptance,
) -> Result<PendingResult, AppError> {
    if let Some(existing) = user::get_user_by_email(state, email).await {
        // An active account, or one whose display name is now taken by someone
        // else. Either way the honest answer would confirm the address to whoever
        // asked. Answer exactly as for a fresh signup and mail the owner instead.
        let name_taken = user::get_user_by_name(state, name)
            .await
            .is_some_and(|owner| owner.id != existing.id);
        if existing.status_id != 2 || name_taken {
            // Same Argon2id cost the real path pays, so the decoy is also
            // indistinguishable by response time.
            let _ = security::hash_password(password.to_string());
            let sent = send_already_registered_notice(email).await.is_ok();
            return Ok(PendingResult {
                email: email.into(),
                accepted_deliveries: u8::from(sent),
                resend_available_at: now().timestamp() + COOLDOWN_SECS,
                delivery_failed: !sent,
                reactivating: false,
            });
        }
        // A reactivation is a fresh agreement to today's documents.
        crate::users::legal::record(state, &existing.id, acceptance).await?;
        return begin_verification(
            state,
            email,
            VerificationState {
                user_id: existing.id,
                code_digest: String::new(),
                code_attempts: 0,
                accepted_deliveries: 0,
                resend_available_at: now().timestamp(),
                kind: VerificationKind::Reactivation {
                    hashed_password: security::hash_password(password.to_string()),
                    name: name.to_string(),
                },
            },
            // Never `true` on the wire: telling the browser "this is a
            // reactivation" tells it the address has a deactivated account.
            false,
        )
        .await;
    }
    if user::get_user_by_name(state, name).await.is_some() {
        return Err(AppError::AlreadyExists);
    }

    let user_id = Uuid::now_v7().to_string();
    user::create_pending_cloud_user(
        state,
        &user_id,
        email,
        &security::hash_password(password.to_string()),
        name,
        &deadline(),
    )
    .await
    .map_err(AppError::DatabaseError)?;
    if let Err(error) = crate::users::legal::record(state, &user_id, acceptance).await {
        let _ = user::delete_pending_user(state, &user_id).await;
        return Err(error);
    }
    let result = begin_verification(
        state,
        email,
        VerificationState {
            user_id: user_id.clone(),
            code_digest: String::new(),
            code_attempts: 0,
            accepted_deliveries: 0,
            resend_available_at: now().timestamp(),
            kind: VerificationKind::Registration,
        },
        false,
    )
    .await;
    if result.is_err() {
        let _ = user::delete_pending_user(state, &user_id).await;
    }
    result
}

async fn begin_verification(
    state: &SharedState,
    email: &str,
    initial: VerificationState,
    reactivating: bool,
) -> Result<PendingResult, AppError> {
    let raw = issue(state, &initial).await?;
    let sent = send(state, email, &raw).await.is_ok();
    let mut persisted: VerificationState = state
        .ephemeral
        .get_json(&user_key(&initial.user_id))
        .await
        .map_err(AppError::InternalError)?
        .ok_or_else(|| AppError::InternalError("verification state disappeared".into()))?;
    persisted.accepted_deliveries = u8::from(sent);
    persisted.resend_available_at = now().timestamp() + COOLDOWN_SECS;
    state
        .ephemeral
        .set_json(&user_key(&initial.user_id), &persisted, TTL_SECS)
        .await
        .map_err(AppError::InternalError)?;
    Ok(PendingResult {
        email: email.into(),
        accepted_deliveries: persisted.accepted_deliveries,
        resend_available_at: persisted.resend_available_at,
        delivery_failed: !sent,
        reactivating,
    })
}

pub async fn resend(state: &SharedState, email: &str) -> Result<(), AppError> {
    let Some(row) = user::get_user_by_email(state, email).await else {
        return Ok(());
    };
    if !matches!(row.status_id, 2 | 3) {
        return Ok(());
    }
    if !state
        .ephemeral
        .set_json_if_absent(&lock_key(&row.id), &true, 30)
        .await
        .map_err(AppError::InternalError)?
    {
        return Ok(());
    }
    let result = async {
        let Some(old) = state
            .ephemeral
            .get_json::<VerificationState>(&user_key(&row.id))
            .await
            .map_err(AppError::InternalError)?
        else {
            return Ok(());
        };
        if old.accepted_deliveries >= MAX_DELIVERIES || old.resend_available_at > now().timestamp()
        {
            return Ok(());
        }
        let code = generate_code();
        if send(state, email, &code).await.is_err() {
            return Ok(());
        }
        let code_digest = digest(&code);
        let next = VerificationState {
            user_id: row.id.clone(),
            code_digest: code_digest.clone(),
            code_attempts: 0,
            accepted_deliveries: old.accepted_deliveries + 1,
            resend_available_at: now().timestamp() + COOLDOWN_SECS,
            kind: old.kind,
        };
        let _ = state.ephemeral.delete(&code_key(&old.code_digest)).await;
        state
            .ephemeral
            .set_json(&code_key(&code_digest), &row.id, TTL_SECS)
            .await
            .map_err(AppError::InternalError)?;
        state
            .ephemeral
            .set_json(&user_key(&row.id), &next, TTL_SECS)
            .await
            .map_err(AppError::InternalError)?;
        if row.status_id == 3 {
            user::update_email_verification_expiry(state, &row.id, &deadline())
                .await
                .map_err(AppError::DatabaseError)?;
        }
        Ok(())
    }
    .await;
    let _ = state.ephemeral.delete(&lock_key(&row.id)).await;
    result
}

pub async fn confirm(
    state: &SharedState,
    email: &str,
    raw_code: &str,
) -> Result<ConfirmResult, AppError> {
    let code = normalize_code(raw_code)?;

    let count = state
        .ephemeral
        .fixed_window_increment(&attempts_key(email), PER_EMAIL_WINDOW)
        .await
        .map_err(|e| AppError::InternalError(e))?;
    if count > PER_EMAIL_MAX {
        return Err(AppError::ValidationError(
            "Too many verification attempts. Try again later.".into(),
        ));
    }

    let code_digest = digest(&code);
    let Some(user_id) = state
        .ephemeral
        .get_json::<String>(&code_key(&code_digest))
        .await
        .map_err(AppError::InternalError)?
    else {
        return Err(AppError::ValidationError(
            "Invalid or expired verification code.".into(),
        ));
    };

    let Some(mut verification) = state
        .ephemeral
        .get_json::<VerificationState>(&user_key(&user_id))
        .await
        .map_err(AppError::InternalError)?
    else {
        return Err(AppError::ValidationError(
            "Invalid or expired verification code.".into(),
        ));
    };

    if verification.code_attempts >= MAX_CODE_ATTEMPTS {
        let _ = state.ephemeral.delete(&code_key(&code_digest)).await;
        return Err(AppError::ValidationError(
            "Too many wrong attempts. Request a new code.".into(),
        ));
    }

    if verification.code_digest != code_digest {
        verification.code_attempts += 1;
        state
            .ephemeral
            .set_json(&user_key(&user_id), &verification, TTL_SECS)
            .await
            .map_err(AppError::InternalError)?;
        if verification.code_attempts >= MAX_CODE_ATTEMPTS {
            let _ = state.ephemeral.delete(&code_key(&code_digest)).await;
            return Err(AppError::ValidationError(
                "Too many wrong attempts. Request a new code.".into(),
            ));
        }
        return Err(AppError::ValidationError(
            "Invalid verification code.".into(),
        ));
    }

    let Some(row) = user::get_user_by_id(state, &user_id).await else {
        return Err(AppError::ValidationError(
            "Invalid or expired verification code.".into(),
        ));
    };
    let now_string = now().naive_utc().format("%Y-%m-%d %H:%M:%S").to_string();
    let reactivated = crate::git::repositories::db::retry_on_stale_snapshot(|| async {
        let conn = crate::git::repositories::db::get_conn(state).await;
        let mut tx = conn
            .begin()
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        let reactivated = match &verification.kind {
            VerificationKind::Registration => {
                let Some(expires) = row.email_verification_expires_at.as_deref() else {
                    return Err(AppError::ValidationError(
                        "Invalid or expired verification code.".into(),
                    ));
                };
                if row.status_id != 3 || expires <= now_string.as_str() {
                    return Err(AppError::ValidationError(
                        "Invalid or expired verification code.".into(),
                    ));
                }
                let updated = sqlx::query("UPDATE users SET status_id = 1, email_verification_expires_at = NULL WHERE id = ?1 AND status_id = 3 AND email_verification_expires_at > ?2")
                    .bind(user_id.clone())
                    .bind(now_string.clone())
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| AppError::DatabaseError(e.to_string()))?
                    .rows_affected();
                if updated == 0 {
                    return Err(AppError::ValidationError("Invalid or expired verification code.".into()));
                }
                let organization_id = Uuid::now_v7().to_string();
                organization::create_organization_on(
                    &mut *tx,
                    &organization_id,
                    &format!("{}'s Org", row.name),
                    &format!("organization-{organization_id}"),
                    "",
                    "shield",
                    "",
                    &row.name,
                    &user_id,
                )
                .await
                .map_err(AppError::DatabaseError)?;
                organization_member::add_member_on(&mut *tx, &organization_id, &user_id, 1)
                    .await
                    .map_err(AppError::DatabaseError)?;
                sqlx::query("INSERT INTO user_global_roles (user_id, role_id) SELECT ?1, 0 WHERE NOT EXISTS (SELECT 1 FROM user_global_roles WHERE role_id = 0)")
                    .bind(user_id.clone())
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| AppError::DatabaseError(e.to_string()))?;
                false
            }
            VerificationKind::Reactivation {
                hashed_password,
                name,
            } => {
                if row.status_id != 2 {
                    return Err(AppError::ValidationError(
                        "Invalid or expired verification code.".into(),
                    ));
                }
                let updated = sqlx::query("UPDATE users SET status_id = 1, name = ?1, hashed_password = ?2, deactivated_at = NULL WHERE id = ?3 AND status_id = 2")
                    .bind(name.clone())
                    .bind(hashed_password.clone())
                    .bind(user_id.clone())
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| AppError::DatabaseError(e.to_string()))?
                    .rows_affected();
                if updated == 0 {
                    return Err(AppError::ValidationError("Invalid or expired verification code.".into()));
                }
                true
            }
        };
        tx.commit()
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        Ok(reactivated)
    })
    .await?;
    let _ = state.ephemeral.delete(&user_key(&user_id)).await;
    let _ = state.ephemeral.delete(&code_key(&code_digest)).await;
    Ok(ConfirmResult {
        verified: true,
        reactivated,
    })
}

pub async fn cleanup(state: &SharedState) -> Result<usize, AppError> {
    let now_string = now().naive_utc().format("%Y-%m-%d %H:%M:%S").to_string();
    let ids = user::list_expired_pending_user_ids(state, &now_string)
        .await
        .map_err(AppError::DatabaseError)?;
    let mut count = 0;
    for id in ids {
        if user::delete_pending_user(state, &id)
            .await
            .map_err(AppError::DatabaseError)?
        {
            count += 1;
            let _ = state.ephemeral.delete(&user_key(&id)).await;
        }
    }
    Ok(count)
}

pub fn spawn_cleanup(state: SharedState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Err(error) = cleanup(&state).await {
                tracing::warn!(%error, "could not clean expired pending users");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_generates_6_chars() {
        for _ in 0..100 {
            let code = generate_code();
            assert_eq!(code.len(), 6, "code length: {code}");
            assert!(
                code.chars().all(|c| c.is_ascii_alphanumeric()),
                "code has non-alphanum: {code}"
            );
        }
    }

    #[test]
    fn code_has_4_digits_2_letters() {
        for _ in 0..100 {
            let code = generate_code();
            let digits = code.chars().filter(|c| c.is_ascii_digit()).count();
            let letters = code.chars().filter(|c| c.is_ascii_uppercase()).count();
            assert_eq!(digits, 4, "code {code}: expected 4 digits, got {digits}");
            assert_eq!(letters, 2, "code {code}: expected 2 letters, got {letters}");
        }
    }

    #[test]
    fn code_is_uppercase() {
        let code = generate_code();
        assert_eq!(code, code.to_ascii_uppercase());
    }

    #[test]
    fn digest_stable() {
        let d1 = digest("4B72K9");
        let d2 = digest("4B72K9");
        assert_eq!(d1, d2);
    }

    #[test]
    fn digest_different_for_different_inputs() {
        let d1 = digest("4B72K9");
        let d2 = digest("4B72K8");
        assert_ne!(d1, d2);
    }

    #[test]
    fn normalize_code_strips_spaces() {
        let result = normalize_code("4B 72 K9").unwrap();
        assert_eq!(result, "4B72K9");
    }

    #[test]
    fn normalize_code_strips_dashes() {
        let result = normalize_code("4B-72-K9").unwrap();
        assert_eq!(result, "4B72K9");
    }

    #[test]
    fn normalize_code_lowercase_to_uppercase() {
        let result = normalize_code("4b72k9").unwrap();
        assert_eq!(result, "4B72K9");
    }

    #[test]
    fn normalize_code_rejects_too_short() {
        assert!(normalize_code("4B72K").is_err());
    }

    #[test]
    fn normalize_code_rejects_too_long() {
        assert!(normalize_code("4B72K9X").is_err());
    }

    #[test]
    fn normalize_code_rejects_non_alphanum_only() {
        assert!(normalize_code("!@#$%^").is_err());
    }
}
