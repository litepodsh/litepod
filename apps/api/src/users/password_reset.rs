//! Forgotten-password recovery.
//!
//! Until now the API had no way back in for someone who forgot their password —
//! `grep 'password.reset\|forgot\|reset_token'` returned nothing, and the login
//! page's "Forgot your password?" link pointed at `##`. An account whose password
//! was lost was simply lost.
//!
//! Shape of the flow, and why:
//!
//! * **The request endpoint always answers the same.** Saying "no account with
//!   that address" would turn it into an account-enumeration oracle, which is
//!   exactly the leak M1 closes elsewhere. Whether or not an email goes out is
//!   invisible to the caller.
//! * **Only the digest of the token is stored**, in the ephemeral store, keyed by
//!   that digest. A dump of the store cannot be replayed into an account. Same
//!   pattern as invitation tokens and email verification codes.
//! * **The token is single-use** — `take_json` is a `GETDEL`, so two concurrent
//!   confirmations cannot both succeed.
//! * **Confirming revokes every session**, including the requester's own. If the
//!   reset is a response to a compromise, the intruder's session has to go, and
//!   we cannot tell which session that is.
//! * **SSO-only accounts get nothing.** An account created by SSO/SCIM has no
//!   password; giving it one through email would bypass the IdP, including for
//!   someone the IdP already removed. The answer is the same as for any address.
//! * **2FA still applies.** A reset restores the password, which is the first
//!   factor. Accounts with TOTP enabled still face the challenge at login.

use serde::{Deserialize, Serialize};

use crate::dtos::response::AppError;
use crate::git::repositories::{session, user};
use crate::state::state::SharedState;
use crate::utils::security;

/// Long enough to find the mail and act on it, short enough that a link left in
/// an inbox does not stay a live credential.
const TTL_SECS: u64 = 3600;

fn token_key(digest: &str) -> String {
    format!("password-reset:v1:token:{digest}")
}

/// One outstanding request per account, so repeatedly hitting the endpoint
/// cannot flood someone's inbox.
fn cooldown_key(user_id: &str) -> String {
    format!("password-reset:v1:cooldown:{user_id}")
}
const COOLDOWN_SECS: u64 = 120;

#[derive(Serialize, Deserialize)]
struct ResetRecord {
    user_id: String,
}

/// Active, and not created by SSO/SCIM: for those the IdP is the only way in,
/// and a reset would let someone the IdP removed back in through their mailbox.
fn may_reset(user_row: &user::UserRow) -> bool {
    user::is_active(user_row) && user_row.hashed_password != crate::ee::sso::NO_PASSWORD
}

/// Begin a reset. Always `Ok(())` for a well-formed request, whatever the
/// address turns out to be.
pub async fn request(
    state: &SharedState,
    email: &str,
    public_base_url: &str,
) -> Result<(), AppError> {
    let Some(user_row) = user::get_user_by_email(state, email).await else {
        return Ok(());
    };
    if !may_reset(&user_row) {
        return Ok(());
    }

    // `set_json_if_absent` returning false means a request for this account is
    // already in flight. Still answer Ok — the caller learns nothing either way.
    let fresh = state
        .ephemeral
        .set_json_if_absent(&cooldown_key(&user_row.id), &true, COOLDOWN_SECS)
        .await
        .unwrap_or(true);
    if !fresh {
        return Ok(());
    }

    let token = security::generate_token();
    state
        .ephemeral
        .set_json(
            &token_key(&security::digest_token(&token)),
            &ResetRecord {
                user_id: user_row.id,
            },
            TTL_SECS,
        )
        .await
        .map_err(AppError::InternalError)?;

    let link = format!(
        "{}/reset-password?token={token}",
        public_base_url.trim_end_matches('/')
    );
    let html = format!(
        "<p>Someone asked to reset the password for your litepod account.</p>\
         <p><a href=\"{link}\">Choose a new password</a></p>\
         <p>The link works once and expires in one hour. \
         If you didn't ask for this, ignore this message — nothing has changed.</p>"
    );
    // A delivery failure must not change the answer, or the response time and
    // shape start distinguishing real addresses again.
    let _ =
        crate::users::email_verification::deliver(email, "Reset your litepod password", html).await;
    Ok(())
}

/// Finish a reset: spend the token, set the password, drop every session.
pub async fn confirm(
    state: &SharedState,
    token: &str,
    new_password: &str,
    confirm_password: &str,
) -> Result<(), AppError> {
    if new_password != confirm_password {
        return Err(AppError::ValidationError("Passwords do not match".into()));
    }
    if new_password.len() < 8 {
        return Err(AppError::ValidationError(
            "Password must be at least 8 characters".into(),
        ));
    }

    // GETDEL: the token is spent by reading it, so a second confirmation with the
    // same link finds nothing.
    let record: ResetRecord = state
        .ephemeral
        .take_json(&token_key(&security::digest_token(token)))
        .await
        .map_err(AppError::InternalError)?
        // Expired, already used, or never issued — one answer for all three.
        .ok_or(AppError::InvalidCredentials)?;

    let user_row = user::get_user_by_id(state, &record.user_id)
        .await
        .ok_or(AppError::InvalidCredentials)?;
    if !user::is_active(&user_row) {
        return Err(AppError::InvalidCredentials);
    }

    let hashed = security::hash_password(new_password.to_string());
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE users SET hashed_password = ?1 WHERE id = ?2")
        .bind(hashed)
        .bind(record.user_id.as_str())
        .execute(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    // Everything, with no exception for the caller: a reset is the response to a
    // possible compromise, and the intruder's session is indistinguishable here.
    if let Err(error) = session::revoke_all_for_user(state, &record.user_id).await {
        tracing::error!(%error, user_id = %record.user_id, "password reset but sessions were not revoked");
    }
    let _ = state.ephemeral.delete(&cooldown_key(&record.user_id)).await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{cleanup_database, test_state};

    #[tokio::test]
    async fn sso_only_accounts_get_no_reset_and_no_password_login() {
        let (state, path) = test_state().await;
        sqlx::query("UPDATE users SET hashed_password = ?1 WHERE id = 'editor'")
            .bind(crate::ee::sso::NO_PASSWORD)
            .execute(&state.db)
            .await
            .unwrap();

        let editor = user::get_user_by_id(&state, "editor").await.unwrap();
        let viewer = user::get_user_by_id(&state, "viewer").await.unwrap();
        assert!(!may_reset(&editor), "SSO-only account");
        assert!(may_reset(&viewer), "password account");
        // Same answer as for any address.
        assert!(
            request(&state, "editor@example.com", "http://litepod.test")
                .await
                .is_ok()
        );

        assert!(
            crate::cqrs::commands::auth::login(
                &state,
                "editor@example.com",
                crate::ee::sso::NO_PASSWORD,
                None
            )
            .await
            .is_err(),
            "the sentinel is not a password"
        );
        cleanup_database(&path);
    }
}
