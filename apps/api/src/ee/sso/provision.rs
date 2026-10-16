//! One path from "the IdP vouched for this person" to a Litepod user, shared
//! by every SSO front-end (OIDC now, SAML later). Spec §3.

use sqlx::SqliteConnection;
use uuid::Uuid;

use super::{Connection, NO_PASSWORD, db, domain_is_verified};
use crate::dtos::response::AppError;

/// What a verified login says about the person.
#[derive(Clone, Debug, Default)]
pub struct Identity {
    /// Stable per-IdP id (OIDC `sub`, SAML persistent NameID).
    pub subject: String,
    pub email: String,
    pub name: String,
    /// External group ids (Entra objectIds), already resolved past overage.
    pub groups: Vec<String>,
}

/// Links (or creates) the user, ensures membership, and sets their IdP-managed
/// groups to exactly the mapped set. Runs on the caller's transaction.
pub async fn provision(
    conn: &mut SqliteConnection,
    connection: &Connection,
    identity: &Identity,
) -> Result<String, AppError> {
    let linked: Option<String> = sqlx::query_scalar(
        "SELECT user_id FROM user_identities WHERE connection_id = ?1 AND subject = ?2",
    )
    .bind(&connection.id)
    .bind(&identity.subject)
    .fetch_optional(&mut *conn)
    .await
    .map_err(db)?;

    let user_id = match linked {
        Some(user_id) => user_id,
        None => link_or_create(conn, connection, identity).await?,
    };

    let status: Option<i64> = sqlx::query_scalar("SELECT status_id FROM users WHERE id = ?1")
        .bind(&user_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(db)?;
    match status {
        Some(1) => {}
        // Invited and never registered: the IdP has just proven the address.
        Some(3) => {
            sqlx::query("UPDATE users SET status_id = 1, email_verification_expires_at = NULL WHERE id = ?1")
                .bind(&user_id)
                .execute(&mut *conn)
                .await
                .map_err(db)?;
        }
        // Deactivated (SCIM or an admin): the IdP saying yes does not undo that.
        _ => return Err(AppError::InvalidCredentials),
    }

    sqlx::query(
        "INSERT OR IGNORE INTO organization_members (organization_id, user_id, role_id) VALUES (?1, ?2, ?3)",
    )
    .bind(&connection.organization_id)
    .bind(&user_id)
    .bind(connection.default_base_role_id)
    .execute(&mut *conn)
    .await
    .map_err(db)?;

    sync_groups(conn, connection, &user_id, &identity.groups).await?;
    Ok(user_id)
}

async fn link_or_create(
    conn: &mut SqliteConnection,
    connection: &Connection,
    identity: &Identity,
) -> Result<String, AppError> {
    let email = identity.email.trim().to_ascii_lowercase();
    // Never link to (or create) an account outside a domain this org proved it
    // owns: otherwise any org's IdP could sign in as anyone@anywhere.
    if !domain_is_verified(conn, &connection.organization_id, &email).await? {
        return Err(AppError::ValidationError(
            "Your email address is not under a domain verified by this organization".into(),
        ));
    }

    let existing: Option<String> = sqlx::query_scalar("SELECT id FROM users WHERE email = ?1")
        .bind(&email)
        .fetch_optional(&mut *conn)
        .await
        .map_err(db)?;
    let user_id = match existing {
        Some(id) => id,
        None if connection.jit_enabled => create_user(conn, &email, &identity.name).await?,
        None => {
            return Err(AppError::ValidationError(
                "Your account has not been provisioned in Litepod yet".into(),
            ));
        }
    };

    sqlx::query(
        "INSERT INTO user_identities (id, user_id, connection_id, subject) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(Uuid::now_v7().to_string())
    .bind(&user_id)
    .bind(&connection.id)
    .bind(&identity.subject)
    .execute(&mut *conn)
    .await
    .map_err(db)?;
    Ok(user_id)
}

/// A new active user with no usable password. `users.name` is unique, so a
/// taken display name falls back to the email (which is unique too).
pub async fn create_user(
    conn: &mut SqliteConnection,
    email: &str,
    name: &str,
) -> Result<String, AppError> {
    let id = Uuid::now_v7().to_string();
    let name = if name.trim().is_empty() {
        email
    } else {
        name.trim()
    };
    let taken: Option<i64> = sqlx::query_scalar("SELECT 1 FROM users WHERE name = ?1")
        .bind(name)
        .fetch_optional(&mut *conn)
        .await
        .map_err(db)?;
    sqlx::query(
        "INSERT INTO users (id, email, hashed_password, name, status_id) VALUES (?1, ?2, ?3, ?4, 1)",
    )
    .bind(&id)
    .bind(email)
    .bind(NO_PASSWORD)
    .bind(if taken.is_some() { email } else { name })
    .execute(&mut *conn)
    .await
    .map_err(db)?;
    Ok(id)
}

/// Membership of the groups this connection maps becomes exactly the groups
/// the IdP sent. Groups the connection does not map are not touched.
async fn sync_groups(
    conn: &mut SqliteConnection,
    connection: &Connection,
    user_id: &str,
    external_groups: &[String],
) -> Result<(), AppError> {
    let external = serde_json::to_string(external_groups).map_err(db)?;
    sqlx::query(
        "DELETE FROM rbac_group_members
          WHERE user_id = ?1
            AND group_id IN (SELECT group_id FROM sso_group_mappings WHERE connection_id = ?2)
            AND group_id NOT IN (SELECT group_id FROM sso_group_mappings
                                  WHERE connection_id = ?2
                                    AND external_group IN (SELECT value FROM json_each(?3)))",
    )
    .bind(user_id)
    .bind(&connection.id)
    .bind(&external)
    .execute(&mut *conn)
    .await
    .map_err(db)?;
    sqlx::query(
        "INSERT OR IGNORE INTO rbac_group_members (group_id, user_id)
         SELECT DISTINCT group_id, ?1 FROM sso_group_mappings
          WHERE connection_id = ?2 AND external_group IN (SELECT value FROM json_each(?3))",
    )
    .bind(user_id)
    .bind(&connection.id)
    .bind(&external)
    .execute(&mut *conn)
    .await
    .map_err(db)?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::testing::{cleanup_database, test_state};

    /// An enabled OIDC connection for organization-a with `contoso.com`
    /// verified and IdP group `g-ops` mapped to Litepod group `ops`.
    pub(crate) async fn seed_connection(state: &crate::state::state::SharedState) -> Connection {
        for sql in [
            "INSERT INTO sso_connections (id, organization_id, kind, name, enabled, issuer, client_id) \
             VALUES ('c1', 'organization-a', 'oidc', 'Entra', 1, 'https://idp.example', 'litepod')",
            "INSERT INTO sso_domains (id, organization_id, domain, verify_token, verified_at) \
             VALUES ('d1', 'organization-a', 'contoso.com', 't', datetime('now'))",
            "INSERT INTO rbac_groups (id, organization_id, name, source) VALUES ('ops', 'organization-a', 'ops', 'idp')",
            "INSERT INTO sso_group_mappings (connection_id, external_group, group_id) VALUES ('c1', 'g-ops', 'ops')",
        ] {
            sqlx::query(sql).execute(&state.db).await.unwrap();
        }
        let mut conn = state.db.acquire().await.unwrap();
        super::super::connection(&mut conn, "c1")
            .await
            .unwrap()
            .unwrap()
    }

    fn ana(groups: &[&str]) -> Identity {
        Identity {
            subject: "sub-ana".into(),
            email: "Ana@contoso.com".into(),
            name: "Ana".into(),
            groups: groups.iter().map(|g| g.to_string()).collect(),
        }
    }

    async fn in_ops(state: &crate::state::state::SharedState, user_id: &str) -> bool {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM rbac_group_members WHERE group_id = 'ops' AND user_id = ?1",
        )
        .bind(user_id)
        .fetch_one(&state.db)
        .await
        .unwrap()
            == 1
    }

    #[tokio::test]
    async fn jit_creates_the_user_and_groups_follow_each_login() {
        let (state, path) = test_state().await;
        let connection = seed_connection(&state).await;
        let mut conn = state.db.acquire().await.unwrap();

        let user_id = provision(&mut conn, &connection, &ana(&["g-ops", "unmapped"]))
            .await
            .unwrap();
        assert!(in_ops(&state, &user_id).await);
        let role: i64 = sqlx::query_scalar(
            "SELECT role_id FROM organization_members WHERE organization_id = 'organization-a' AND user_id = ?1",
        )
        .bind(&user_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(role, 4, "default base role is Member");

        // Second login, no longer in the group: removed. Same user (by subject).
        let again = provision(&mut conn, &connection, &ana(&[])).await.unwrap();
        assert_eq!(again, user_id);
        assert!(!in_ops(&state, &user_id).await);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn an_address_outside_the_verified_domains_is_refused() {
        let (state, path) = test_state().await;
        let connection = seed_connection(&state).await;
        let mut conn = state.db.acquire().await.unwrap();
        // `editor@example.com` exists locally; the IdP must not be able to claim it.
        let identity = Identity {
            subject: "evil".into(),
            email: "editor@example.com".into(),
            ..Identity::default()
        };
        assert!(matches!(
            provision(&mut conn, &connection, &identity).await,
            Err(AppError::ValidationError(_))
        ));
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn without_jit_only_existing_accounts_get_in() {
        let (state, path) = test_state().await;
        let mut connection = seed_connection(&state).await;
        connection.jit_enabled = false;
        let mut conn = state.db.acquire().await.unwrap();
        assert!(provision(&mut conn, &connection, &ana(&[])).await.is_err());

        create_user(&mut conn, "ana@contoso.com", "Ana")
            .await
            .unwrap();
        provision(&mut conn, &connection, &ana(&[])).await.unwrap();
        cleanup_database(&path);
    }
}
