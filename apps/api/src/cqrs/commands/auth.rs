use uuid::Uuid;

use crate::dtos::auth::{AuthResponse, UserClaims};
use crate::dtos::response::AppError;
use crate::git::repositories::{organization, organization_member, session, user};
use crate::state::state::SharedState;
use crate::utils::security;

/// Counter bumped once per account created through `/register`. Exported on `/metrics`.
pub const USER_REGISTRATIONS_METRIC: &str = "litepod_user_registrations_total";

pub async fn register(
    state: &SharedState,
    email: &str,
    password: &str,
    name: &str,
    user_agent: Option<&str>,
) -> Result<(AuthResponse, String), AppError> {
    if state.is_cloud {
        return register_public_cloud(state, email, password, name, user_agent).await;
    }

    if user::count_sysadmins(state).await? > 0 {
        return Err(AppError::RegistrationClosed);
    }

    if user::get_user_by_email(state, email).await.is_some() {
        return Err(AppError::AlreadyExists);
    }

    if user::get_user_by_name(state, name).await.is_some() {
        return Err(AppError::AlreadyExists);
    }

    let user_id = Uuid::now_v7().to_string();
    let hashed = security::hash_password(password.to_string());

    user::create_user(state, &user_id, email, &hashed, name).await?;

    // Lost the race against a concurrent setup request: the instance already has
    // its SysAdmin, so this registration must not proceed as a second one.
    if !user::make_sysadmin(state, &user_id).await? {
        let _ = user::delete_user(state, &user_id).await;
        return Err(AppError::RegistrationClosed);
    }

    let root_organization_id = Uuid::now_v7().to_string();
    organization::create_root_organization(
        state,
        &root_organization_id,
        "Root Org",
        "root",
        "System root organization",
        "shield",
        "System",
        &user_id,
    )
    .await?;

    organization_member::add_member(state, &root_organization_id, &user_id, 1).await?;

    let is_sysadmin = user::is_sysadmin(state, &user_id).await;
    let session_token = create_session(state, &user_id, user_agent, None).await?;
    metrics::counter!(USER_REGISTRATIONS_METRIC).increment(1);

    Ok((
        AuthResponse {
            user: UserClaims {
                user_id,
                email: email.to_string(),
                name: name.to_string(),
                is_sysadmin,
            },
        },
        session_token,
    ))
}

pub async fn register_public_cloud(
    state: &SharedState,
    email: &str,
    password: &str,
    name: &str,
    user_agent: Option<&str>,
) -> Result<(AuthResponse, String), AppError> {
    if user::get_user_by_email(state, email).await.is_some()
        || user::get_user_by_name(state, name).await.is_some()
    {
        return Err(AppError::AlreadyExists);
    }

    let user_id = Uuid::now_v7().to_string();
    let organization_id = Uuid::now_v7().to_string();
    let hashed = security::hash_password(password.to_string());

    let is_sysadmin = crate::git::repositories::db::retry_on_stale_snapshot(|| async {
        let conn = crate::git::repositories::db::get_conn(state).await;
        let mut tx = conn.begin().await?;

        sqlx::query("INSERT INTO users (id, email, hashed_password, name) VALUES (?1, ?2, ?3, ?4)")
            .bind(user_id.as_str())
            .bind(email)
            .bind(hashed.as_str())
            .bind(name)
            .execute(&mut *tx)
            .await?;
        // Conditional insert instead of count-then-insert: only the very first cloud
        // account can become SysAdmin, and never two of them under concurrency.
        let is_sysadmin = sqlx::query(
            "INSERT INTO user_global_roles (user_id, role_id)
                 SELECT ?1, 0
                 WHERE NOT EXISTS (SELECT 1 FROM user_global_roles WHERE role_id = 0)",
        )
        .bind(user_id.as_str())
        .execute(&mut *tx)
        .await?
        .rows_affected()
            > 0;
        organization::create_organization_on(
            &mut *tx,
            &organization_id,
            &format!("{name}'s Org"),
            &format!("organization-{organization_id}"),
            "",
            "shield",
            "",
            name,
            &user_id,
        )
        .await?;
        organization_member::add_member_on(&mut *tx, &organization_id, &user_id, 1).await?;
        tx.commit().await?;
        Ok(is_sysadmin)
    })
    .await?;

    let session_token = create_session(state, &user_id, user_agent, None).await?;
    metrics::counter!(USER_REGISTRATIONS_METRIC).increment(1);
    Ok((
        AuthResponse {
            user: UserClaims {
                user_id,
                email: email.to_string(),
                name: name.to_string(),
                is_sysadmin,
            },
        },
        session_token,
    ))
}

/// How long a half-finished login may sit waiting for its second factor.
const TOTP_CHALLENGE_TTL_SECS: u64 = 300;
/// Wrong codes allowed against one challenge before it is thrown away. Without a
/// cap, a five-minute window is long enough to walk a good part of a six-digit
/// space.
const TOTP_CHALLENGE_MAX_ATTEMPTS: u8 = 5;

fn totp_challenge_key(digest: &str) -> String {
    format!("login:v1:totp-challenge:{digest}")
}

#[derive(serde::Serialize, serde::Deserialize)]
struct TotpChallenge {
    user_id: String,
    attempts: u8,
}

/// What `login` returns: either a finished session, or a demand for the second
/// factor. Password-only callers cannot turn the second into the first.
pub enum LoginOutcome {
    Authenticated(Box<(AuthResponse, String)>),
    TotpRequired { challenge_token: String },
}

pub async fn login(
    state: &SharedState,
    email: &str,
    password: &str,
    user_agent: Option<&str>,
) -> Result<LoginOutcome, AppError> {
    // Every failing branch below must cost the same as a successful one, or the
    // response time tells an attacker which addresses have accounts. `verify_password`
    // is a full Argon2id verification; the dummy burns the identical work when
    // there is no stored hash to check against.
    let Some(user_row) = user::get_user_by_email(state, email).await else {
        security::verify_password_dummy(password.as_bytes());
        return Err(AppError::InvalidCredentials);
    };

    if !user::is_active(&user_row) {
        security::verify_password_dummy(password.as_bytes());
        return Err(AppError::InvalidCredentials);
    }

    if !security::verify_password(&user_row.hashed_password, password.as_bytes()) {
        return Err(AppError::InvalidCredentials);
    }

    // Checked only after the password verified, so the refusal never tells a
    // stranger an account exists. Before the TOTP step, so a 2FA user is
    // refused too. The SysAdmin is the break-glass; cloud has no SSO at all.
    if !state.is_cloud && !user::is_sysadmin(state, &user_row.id).await {
        let mut conn = state
            .db
            .acquire()
            .await
            .map_err(|e| AppError::DatabaseError(e.to_string()))?;
        if crate::ee::sso::sso_required(&mut conn, &user_row.id, &user_row.email).await? {
            return Err(AppError::SsoRequired);
        }
    }

    // The password is correct, but for a 2FA account it is only the first half.
    // No session is created here — the caller gets an opaque challenge and has to
    // come back through `complete_totp_login` with a code.
    if crate::git::repositories::account::is_totp_enabled(state, &user_row.id).await? {
        let challenge_token = security::generate_token();
        state
            .ephemeral
            .set_json(
                &totp_challenge_key(&security::digest_token(&challenge_token)),
                &TotpChallenge {
                    user_id: user_row.id.clone(),
                    attempts: 0,
                },
                TOTP_CHALLENGE_TTL_SECS,
            )
            .await
            .map_err(AppError::InternalError)?;
        return Ok(LoginOutcome::TotpRequired { challenge_token });
    }

    let is_sysadmin = user::is_sysadmin(state, &user_row.id).await;
    let session_token = create_session(state, &user_row.id, user_agent, None).await?;

    Ok(LoginOutcome::Authenticated(Box::new((
        AuthResponse {
            user: UserClaims {
                user_id: user_row.id,
                email: user_row.email,
                name: user_row.name,
                is_sysadmin,
            },
        },
        session_token,
    ))))
}

/// Second half of a 2FA login: exchange a challenge plus a TOTP or recovery code
/// for a session.
///
/// The challenge is stored by digest, so the ephemeral store never holds a value
/// that would let its contents be replayed as a login. It is consumed on success
/// and on running out of attempts, so it is strictly single-use.
pub async fn complete_totp_login(
    state: &SharedState,
    challenge_token: &str,
    code: &str,
    user_agent: Option<&str>,
) -> Result<(AuthResponse, String, &'static str), AppError> {
    let key = totp_challenge_key(&security::digest_token(challenge_token));
    let challenge: TotpChallenge = state
        .ephemeral
        .get_json(&key)
        .await
        .map_err(AppError::InternalError)?
        // Expired, already used, or never existed — all the same answer, so a
        // caller cannot tell a stale challenge from a forged one.
        .ok_or(AppError::InvalidCredentials)?;

    let Ok(mfa_method) =
        crate::cqrs::commands::account::verify_second_factor(state, &challenge.user_id, code).await
    else {
        let attempts = challenge.attempts + 1;
        if attempts >= TOTP_CHALLENGE_MAX_ATTEMPTS {
            let _ = state.ephemeral.delete(&key).await;
        } else {
            // Keep the original deadline rather than extending it: a wrong code
            // must not buy more time.
            let ttl = state
                .ephemeral
                .remaining_ttl(&key)
                .await
                .ok()
                .flatten()
                .unwrap_or(TOTP_CHALLENGE_TTL_SECS);
            let _ = state
                .ephemeral
                .set_json(
                    &key,
                    &TotpChallenge {
                        user_id: challenge.user_id,
                        attempts,
                    },
                    ttl,
                )
                .await;
        }
        return Err(AppError::TotpInvalidCode);
    };

    let _ = state.ephemeral.delete(&key).await;

    let user_row = user::get_user_by_id(state, &challenge.user_id)
        .await
        .ok_or(AppError::InvalidCredentials)?;
    if !user::is_active(&user_row) {
        return Err(AppError::InvalidCredentials);
    }

    let is_sysadmin = user::is_sysadmin(state, &user_row.id).await;
    let session_token = create_session(state, &user_row.id, user_agent, None).await?;
    Ok((
        AuthResponse {
            user: UserClaims {
                user_id: user_row.id,
                email: user_row.email,
                name: user_row.name,
                is_sysadmin,
            },
        },
        session_token,
        mfa_method,
    ))
}

pub async fn logout(state: &SharedState, token: &str) -> Result<(), AppError> {
    let _ = session::revoke_session(state, token).await;
    Ok(())
}

pub(crate) async fn create_session(
    state: &SharedState,
    user_id: &str,
    user_agent: Option<&str>,
    sso: Option<&crate::ee::sso::SsoSession>,
) -> Result<String, AppError> {
    let session_id = Uuid::now_v7().to_string();
    // The row id stays a UUIDv7 — it is a database key and being time-ordered is
    // useful there. The token is a bearer credential and gets 256 CSPRNG bits.
    let token = security::generate_token();
    // SSO sessions last as long as their connection says; password ones 3 days.
    let lifetime = sso.map_or(chrono::Duration::days(3), |s| {
        chrono::Duration::minutes(s.ttl_minutes)
    });
    let expires_at = chrono::Utc::now()
        .naive_utc()
        .checked_add_signed(lifetime)
        .expect("valid timestamp")
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();

    match sso {
        Some(sso) => {
            session::create_sso_session(
                state,
                &session_id,
                user_id,
                &token,
                &expires_at,
                user_agent,
                sso,
            )
            .await?
        }
        None => {
            session::create_session(state, &session_id, user_id, &token, &expires_at, user_agent)
                .await?
        }
    }

    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::test_state_for as test_state;

    fn remove_database(path: &std::path::Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    #[tokio::test]
    async fn totp_lookup_database_failure_never_creates_a_session() {
        let (state, path) = test_state(false).await;
        register(&state, "lookup@example.com", "pw", "lookup", None)
            .await
            .unwrap();
        sqlx::query("UPDATE users SET totp_enabled = 1 WHERE email = 'lookup@example.com'")
            .execute(&state.db)
            .await
            .unwrap();
        // The user/password query still works, but the second-factor query fails.
        sqlx::query("ALTER TABLE users RENAME COLUMN totp_enabled TO unavailable_totp_enabled")
            .execute(&state.db)
            .await
            .unwrap();
        let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert!(matches!(
            login(&state, "lookup@example.com", "pw", None).await,
            Err(AppError::DatabaseError(_))
        ));
        let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(before, after);
        remove_database(&path);
    }

    #[tokio::test]
    async fn standard_setup_refuses_a_second_sysadmin() {
        let (state, path) = test_state(false).await;

        let (first, _) = register(&state, "first@example.com", "pw", "first", None)
            .await
            .unwrap();
        assert!(first.user.is_sysadmin);

        let second = register(&state, "second@example.com", "pw", "second", None).await;
        assert!(matches!(second, Err(AppError::RegistrationClosed)));
        assert_eq!(user::count_sysadmins(&state).await.unwrap(), 1);
        assert!(
            user::get_user_by_email(&state, "second@example.com")
                .await
                .is_none()
        );

        remove_database(&path);
    }

    #[tokio::test]
    async fn cloud_signup_grants_sysadmin_only_to_the_first_account() {
        let (state, path) = test_state(true).await;

        let (first, _) = register(&state, "first@example.com", "pw", "first", None)
            .await
            .unwrap();
        assert!(first.user.is_sysadmin);

        let (second, _) = register(&state, "second@example.com", "pw", "second", None)
            .await
            .unwrap();
        assert!(!second.user.is_sysadmin);
        assert!(!user::is_sysadmin(&state, &second.user.user_id).await);
        assert_eq!(user::count_sysadmins(&state).await.unwrap(), 1);

        remove_database(&path);
    }

    /// Guards future code paths: even a direct grant is rejected by the schema.
    #[tokio::test]
    async fn the_database_rejects_a_second_sysadmin_grant() {
        let (state, path) = test_state(true).await;

        let (first, _) = register(&state, "first@example.com", "pw", "first", None)
            .await
            .unwrap();
        let (second, _) = register(&state, "second@example.com", "pw", "second", None)
            .await
            .unwrap();

        assert!(
            !user::make_sysadmin(&state, &second.user.user_id)
                .await
                .unwrap()
        );

        assert!(
            sqlx::query("INSERT INTO user_global_roles (user_id, role_id) VALUES (?1, 0)")
                .bind(second.user.user_id.as_str())
                .execute(&state.db)
                .await
                .is_err()
        );
        assert_eq!(user::count_sysadmins(&state).await.unwrap(), 1);
        assert!(user::is_sysadmin(&state, &first.user.user_id).await);

        remove_database(&path);
    }

    /// Registration still hands back a session directly — only `login` gained the
    /// 2FA branch, and only for accounts that turned it on.
    #[tokio::test]
    async fn a_password_only_account_logs_straight_in() {
        let (state, path) = test_state(false).await;
        register(&state, "solo@example.com", "pw", "solo", None)
            .await
            .unwrap();

        let outcome = login(&state, "solo@example.com", "pw", None).await.unwrap();
        let LoginOutcome::Authenticated(session) = outcome else {
            panic!("an account without TOTP should log in with the password alone");
        };
        let (_, token) = *session;
        // 256 CSPRNG bits, hex-encoded — not the old UUIDv7.
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(uuid::Uuid::parse_str(&token).is_err());

        remove_database(&path);
    }

    /// The point of A1: a correct password must not be enough once TOTP is on.
    ///
    /// Asserted as "not Authenticated" rather than "is TotpRequired" because
    /// issuing the challenge needs the ephemeral store, which these tests run
    /// without. Both outcomes are correct here — the store being unavailable
    /// fails the login closed, it does not fall back to a session.
    #[tokio::test]
    async fn a_totp_account_never_gets_a_session_from_the_password_alone() {
        let (state, path) = test_state(false).await;
        let (registered, _) = register(&state, "tfa@example.com", "pw", "tfa", None)
            .await
            .unwrap();
        crate::git::repositories::account::set_totp_enabled(&state, &registered.user.user_id, true)
            .await
            .unwrap();

        let outcome = login(&state, "tfa@example.com", "pw", None).await;
        assert!(
            !matches!(outcome, Ok(LoginOutcome::Authenticated(_))),
            "password alone issued a session for a 2FA account"
        );

        remove_database(&path);
    }

    #[tokio::test]
    async fn a_wrong_password_is_refused_whether_or_not_the_account_exists() {
        let (state, path) = test_state(false).await;
        register(&state, "real@example.com", "pw", "real", None)
            .await
            .unwrap();

        assert!(matches!(
            login(&state, "real@example.com", "wrong", None).await,
            Err(AppError::InvalidCredentials)
        ));
        assert!(matches!(
            login(&state, "ghost@example.com", "wrong", None).await,
            Err(AppError::InvalidCredentials)
        ));

        remove_database(&path);
    }
}
