//! Enterprise identity, self-hosted only.
//! Nothing here runs unless an organization configures a connection, and no
//! route reaching it is registered on cloud.

pub mod dns;
pub mod oidc;
pub mod provision;
#[cfg(feature = "saml")]
pub mod saml;
pub mod scim;

use sqlx::{Row, SqliteConnection};

use crate::dtos::response::AppError;

/// Placed in `users.hashed_password` for accounts created through SSO or SCIM.
/// Not an Argon2 encoding, so `verify_password` always rejects it.
pub const NO_PASSWORD: &str = "!sso";

pub(crate) fn db(e: impl ToString) -> AppError {
    let message = e.to_string();
    if message.to_ascii_lowercase().contains("unique constraint") {
        AppError::AlreadyExists
    } else {
        AppError::DatabaseError(message)
    }
}

#[derive(Clone, Debug)]
pub struct Connection {
    pub id: String,
    pub organization_id: String,
    pub kind: String,
    pub name: String,
    pub enabled: bool,
    pub issuer: Option<String>,
    pub client_id: Option<String>,
    pub client_secret_encrypted: Option<String>,
    pub groups_claim: String,
    pub jit_enabled: bool,
    pub default_base_role_id: i64,
    /// SAML: the IdP metadata XML and its entity id.
    pub idp_metadata_xml: Option<String>,
    pub idp_entity_id: Option<String>,
    /// Lifetime of a Litepod session created through this connection.
    pub session_ttl_minutes: i64,
}

const COLUMNS: &str = "id, organization_id, kind, name, enabled, issuer, client_id, \
     client_secret_encrypted, groups_claim, jit_enabled, default_base_role_id, \
     idp_metadata_xml, idp_entity_id, session_ttl_minutes";

fn from_row(row: &sqlx::sqlite::SqliteRow) -> Connection {
    Connection {
        id: row.get(0),
        organization_id: row.get(1),
        kind: row.get(2),
        name: row.get(3),
        enabled: row.get::<i64, _>(4) == 1,
        issuer: row.get(5),
        client_id: row.get(6),
        client_secret_encrypted: row.get(7),
        groups_claim: row.get(8),
        jit_enabled: row.get::<i64, _>(9) == 1,
        default_base_role_id: row.get(10),
        idp_metadata_xml: row.get(11),
        idp_entity_id: row.get(12),
        session_ttl_minutes: row.get(13),
    }
}

pub async fn connection(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Option<Connection>, AppError> {
    let row = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM sso_connections WHERE id = ?1"
    ))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(db)?;
    Ok(row.as_ref().map(from_row))
}

pub async fn connections_of(
    conn: &mut SqliteConnection,
    organization_id: &str,
) -> Result<Vec<Connection>, AppError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLUMNS} FROM sso_connections WHERE organization_id = ?1 ORDER BY created_at"
    ))
    .bind(organization_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(db)?;
    Ok(rows.iter().map(from_row).collect())
}

/// Spec §2 start: the IdP URL for the connection serving `email`. An address
/// no enabled connection serves gets the same error as a wrong password, so
/// this cannot be used to learn which domains are configured. `forced` asks
/// the IdP for credentials even with a live IdP session
/// (`prompt=login` / `ForceAuthn`).
pub async fn start_login(
    state: &crate::state::state::SharedState,
    email: &str,
    public_base_url: &str,
    forced: bool,
) -> Result<String, AppError> {
    let mut conn = state.db.acquire().await.map_err(db)?;
    let connection = connection_for_email(&mut conn, email)
        .await?
        .ok_or(AppError::InvalidCredentials)?;
    drop(conn);
    match connection.kind.as_str() {
        "oidc" => oidc::start(state, &connection, email, public_base_url, forced).await,
        #[cfg(feature = "saml")]
        "saml" => saml::start(state, &connection, email, public_base_url, forced).await,
        _ => Err(AppError::InvalidCredentials),
    }
}

/// An in-flight login, as stored by `save_login_state`.
#[derive(Clone, Debug)]
pub struct LoginState {
    pub connection_id: String,
    /// OIDC nonce, or SAML request id.
    pub nonce: String,
    pub pkce_verifier: String,
    /// The address typed on the login page (trimmed, lower-cased).
    pub login_hint: Option<String>,
    /// The IdP was asked to re-authenticate; the result must prove it did.
    pub forced: bool,
}

/// Consumes an in-flight login (single use, unexpired). Committed before the
/// caller talks to the IdP, so a failed exchange can never be retried with
/// the same state.
pub async fn take_login_state(
    state: &crate::state::state::SharedState,
    state_param: &str,
) -> Result<LoginState, AppError> {
    let mut tx = state.db.begin().await.map_err(db)?;
    let row = sqlx::query(
        "DELETE FROM sso_login_states WHERE id = ?1 AND expires_at >= datetime('now')
         RETURNING connection_id, nonce, pkce_verifier, login_hint, forced",
    )
    .bind(crate::utils::security::digest_token(state_param))
    .fetch_optional(&mut *tx)
    .await
    .map_err(db)?
    .ok_or_else(|| {
        AppError::ValidationError(
            "Single sign-on failed: this sign-in link expired or was already used".into(),
        )
    })?;
    tx.commit().await.map_err(db)?;
    Ok(LoginState {
        connection_id: row.get(0),
        nonce: row.get(1),
        pkce_verifier: row.get(2),
        login_hint: row.get(3),
        forced: row.get::<i64, _>(4) == 1,
    })
}

/// Stores an in-flight login keyed by the digest of `state_param`.
pub async fn save_login_state(
    conn: &mut SqliteConnection,
    state_param: &str,
    connection_id: &str,
    nonce: &str,
    pkce_verifier: &str,
    login_hint: Option<&str>,
    forced: bool,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM sso_login_states WHERE expires_at < datetime('now')")
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    sqlx::query(
        "INSERT INTO sso_login_states
            (id, connection_id, nonce, pkce_verifier, expires_at, login_hint, forced)
         VALUES (?1, ?2, ?3, ?4, datetime('now', '+10 minutes'), ?5, ?6)",
    )
    .bind(crate::utils::security::digest_token(state_param))
    .bind(connection_id)
    .bind(nonce)
    .bind(pkce_verifier)
    .bind(
        login_hint
            .map(|h| h.trim().to_ascii_lowercase())
            .filter(|h| !h.is_empty()),
    )
    .bind(forced as i64)
    .execute(&mut *conn)
    .await
    .map_err(db)?;
    Ok(())
}

/// Shown when a forced re-authentication came back without proof the IdP
/// asked for credentials.
pub const REAUTH_REFUSED: &str = "Single sign-on failed: your identity provider did not ask for \
     your credentials again. Sign out of it and try again.";

/// How long ago (seconds) a forced login may have authenticated.
const REAUTH_MAX_AGE_SECS: i64 = 300;
/// Clock skew tolerated for an authentication time in the future.
const REAUTH_SKEW_SECS: i64 = 60;

pub enum Gate {
    /// Sign in (provision) this identity.
    Proceed,
    /// Ask the IdP again, forcing credentials.
    Reauth,
    /// A forced login without proof of fresh credentials: fail closed.
    Refuse,
}

/// The one security decision after the IdP answered, shared by OIDC and SAML.
/// `authenticated_at` is OIDC `auth_time` or SAML `AuthnInstant`.
///
/// A live IdP session answers for whoever owns it, not for the address typed
/// on the login page, so a mismatch is re-asked with credentials forced. A
/// forced login is only accepted with proof the IdP really asked: an IdP that
/// ignores `prompt=login` would otherwise sign the next person in as the last.
pub fn gate(
    login: &LoginState,
    email: &str,
    authenticated_at: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
) -> Gate {
    if login.forced {
        let fresh = authenticated_at.is_some_and(|at| {
            let age = (now - at).num_seconds();
            (-REAUTH_SKEW_SECS..=REAUTH_MAX_AGE_SECS).contains(&age)
        });
        return if fresh { Gate::Proceed } else { Gate::Refuse };
    }
    match login.login_hint.as_deref() {
        Some(hint) if !hint.eq_ignore_ascii_case(email.trim()) => Gate::Reauth,
        _ => Gate::Proceed,
    }
}

/// Where a signed-in SSO session came from; stored on the session row.
#[derive(Clone, Debug)]
pub struct SsoSession {
    pub connection_id: String,
    pub subject: String,
    pub ttl_minutes: i64,
    /// OIDC raw ID token, encrypted with `security::encrypt_data`. Used only
    /// as `id_token_hint` at logout.
    pub id_token_encrypted: Option<String>,
}

pub enum Finish {
    SignedIn {
        user_id: String,
        session: SsoSession,
    },
    /// Send the browser here (the IdP, forcing credentials).
    Reauth { redirect_url: String },
}

/// The lower-cased domain of an address, if it has one.
pub fn email_domain(email: &str) -> Option<String> {
    let (_, domain) = email.trim().rsplit_once('@')?;
    (!domain.is_empty()).then(|| domain.to_ascii_lowercase())
}

/// The enabled connection (OIDC or SAML) serving `email`'s domain: the domain must be
/// verified by the connection's organization. A domain assigned to a connection
/// uses only that one; an unassigned domain uses the org's oldest enabled one.
pub async fn connection_for_email(
    conn: &mut SqliteConnection,
    email: &str,
) -> Result<Option<Connection>, AppError> {
    let Some(domain) = email_domain(email) else {
        return Ok(None);
    };
    let columns = COLUMNS
        .split(", ")
        .map(|c| format!("c.{}", c.trim()))
        .collect::<Vec<_>>()
        .join(", ");
    let row = sqlx::query(&format!(
        "SELECT {columns} FROM sso_domains d
           JOIN sso_connections c ON c.organization_id = d.organization_id
                AND (d.connection_id IS NULL OR c.id = d.connection_id)
          WHERE d.domain = ?1 AND d.verified_at IS NOT NULL
            AND c.enabled = 1
          ORDER BY c.created_at LIMIT 1"
    ))
    .bind(domain)
    .fetch_optional(&mut *conn)
    .await
    .map_err(db)?;
    Ok(row.as_ref().map(from_row))
}

/// Whether `email` sits under a domain `organization_id` has verified.
pub async fn domain_is_verified(
    conn: &mut SqliteConnection,
    organization_id: &str,
    email: &str,
) -> Result<bool, AppError> {
    let Some(domain) = email_domain(email) else {
        return Ok(false);
    };
    let found: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM sso_domains WHERE organization_id = ?1 AND domain = ?2 AND verified_at IS NOT NULL",
    )
    .bind(organization_id)
    .bind(domain)
    .fetch_optional(&mut *conn)
    .await
    .map_err(db)?;
    Ok(found.is_some())
}

/// Spec §5: a password login is refused when the user is a member of an org
/// that enforces SSO and their address is under that org's verified domain.
/// Callers skip this for the SysAdmin (break-glass) and on cloud.
pub async fn sso_required(
    conn: &mut SqliteConnection,
    user_id: &str,
    email: &str,
) -> Result<bool, AppError> {
    let Some(domain) = email_domain(email) else {
        return Ok(false);
    };
    let found: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM organizations o
           JOIN organization_members m ON m.organization_id = o.id AND m.user_id = ?1
           JOIN sso_domains d ON d.organization_id = o.id AND d.domain = ?2 AND d.verified_at IS NOT NULL
          WHERE o.sso_enforced = 1
          LIMIT 1",
    )
    .bind(user_id)
    .bind(domain)
    .fetch_optional(&mut *conn)
    .await
    .map_err(db)?;
    Ok(found.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login(hint: Option<&str>, forced: bool) -> LoginState {
        LoginState {
            connection_id: "c1".into(),
            nonce: "n".into(),
            pkce_verifier: "v".into(),
            login_hint: hint.map(str::to_string),
            forced,
        }
    }

    #[test]
    fn gate_decides_reauth_and_refuses_stale_forced_logins() {
        let now = chrono::Utc::now();
        let fresh = Some(now - chrono::Duration::seconds(30));
        let stale = Some(now - chrono::Duration::minutes(10));
        let future = Some(now + chrono::Duration::minutes(5));
        let ana = "ana@contoso.com";

        // Not forced: only a mismatch triggers re-auth; case and spaces do not.
        assert!(matches!(
            gate(&login(Some(ana), false), "Ana@Contoso.com ", None, now),
            Gate::Proceed
        ));
        assert!(matches!(
            gate(&login(Some("eve@contoso.com"), false), ana, None, now),
            Gate::Reauth
        ));
        assert!(matches!(
            gate(&login(None, false), ana, None, now),
            Gate::Proceed
        ));

        // Forced: identity accepted only with a fresh authentication time.
        assert!(matches!(
            gate(&login(Some("eve@contoso.com"), true), ana, fresh, now),
            Gate::Proceed
        ));
        assert!(matches!(
            gate(&login(Some(ana), true), ana, stale, now),
            Gate::Refuse
        ));
        assert!(matches!(
            gate(&login(Some(ana), true), ana, None, now),
            Gate::Refuse
        ));
        assert!(matches!(
            gate(&login(Some(ana), true), ana, future, now),
            Gate::Refuse
        ));
    }

    #[test]
    fn email_domains_are_lower_cased_and_need_an_at() {
        assert_eq!(
            email_domain("Ana@Contoso.COM").as_deref(),
            Some("contoso.com")
        );
        assert_eq!(email_domain("no-domain"), None);
        assert_eq!(email_domain("trailing@"), None);
    }

    #[tokio::test]
    async fn an_assigned_domain_uses_its_connection_not_the_oldest() {
        let (state, path) = crate::testing::test_state().await;
        provision::tests::seed_connection(&state).await;
        sqlx::query(
            "INSERT INTO sso_connections (id, organization_id, kind, name, enabled, issuer, client_id, created_at) \
             VALUES ('c2', 'organization-a', 'oidc', 'Keycloak', 1, 'https://kc.example', 'litepod', datetime('now', '+1 minute'))",
        )
        .execute(&state.db)
        .await
        .unwrap();
        let mut conn = state.db.acquire().await.unwrap();
        let pick = |id: Option<Connection>| id.map(|c| c.id);

        assert_eq!(
            pick(
                connection_for_email(&mut conn, "ana@contoso.com")
                    .await
                    .unwrap()
            )
            .as_deref(),
            Some("c1"),
            "unassigned: oldest enabled"
        );
        sqlx::query("UPDATE sso_domains SET connection_id = 'c2'")
            .execute(&mut *conn)
            .await
            .unwrap();
        assert_eq!(
            pick(
                connection_for_email(&mut conn, "ana@contoso.com")
                    .await
                    .unwrap()
            )
            .as_deref(),
            Some("c2")
        );
        sqlx::query("UPDATE sso_connections SET enabled = 0 WHERE id = 'c2'")
            .execute(&mut *conn)
            .await
            .unwrap();
        assert!(
            connection_for_email(&mut conn, "ana@contoso.com")
                .await
                .unwrap()
                .is_none(),
            "assigned connection disabled: no fallback"
        );
        drop(conn);
        crate::testing::cleanup_database(&path);
    }
}
