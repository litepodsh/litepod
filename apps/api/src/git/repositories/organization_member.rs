use std::collections::HashSet;

use sqlx::Row;

use crate::state::state::SharedState;

pub struct OrganizationMemberRow {
    pub user_id: String,
    pub email: String,
    pub name: String,
    pub role_id: i64,
    pub role_name: String,
    pub joined_at: String,
    pub disabled: bool,
    pub two_factor_enabled: bool,
}

pub struct UserMembershipRow {
    pub organization_id: String,
    pub role_id: i64,
    pub role_name: String,
    pub joined_at: String,
}

pub async fn add_member(
    state: &SharedState,
    organization_id: &str,
    user_id: &str,
    role_id: i64,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    add_member_on(&conn, organization_id, user_id, role_id).await
}

/// Called both standalone (with the pool) and inside a transaction (adding
/// the founding member together with an organization's creation), so it
/// takes a generic executor.
pub async fn add_member_on<'e, E>(
    executor: E,
    organization_id: &str,
    user_id: &str,
    role_id: i64,
) -> Result<(), String>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role_id) VALUES (?1, ?2, ?3)",
    )
    .bind(organization_id)
    .bind(user_id)
    .bind(role_id)
    .execute(executor)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Only ever called inside a transaction, so it takes a generic executor
/// rather than a concrete connection type.
pub async fn list_admin_organization_ids_on<'e, E>(
    executor: E,
    user_id: &str,
) -> Result<HashSet<String>, String>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let rows = sqlx::query(
        "SELECT organization_id FROM organization_members WHERE user_id = ?1 AND role_id = 1",
    )
    .bind(user_id)
    .fetch_all(executor)
    .await
    .map_err(|e| e.to_string())?;
    let mut organization_ids = HashSet::new();
    for row in &rows {
        organization_ids.insert(row.try_get(0).map_err(|e| e.to_string())?);
    }
    Ok(organization_ids)
}

pub async fn is_member(state: &SharedState, organization_id: &str, user_id: &str) -> bool {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT 1 FROM organization_members WHERE organization_id = ?1 AND user_id = ?2",
    )
    .bind(organization_id)
    .bind(user_id)
    .fetch_optional(&conn)
    .await;

    match row {
        Ok(row) => row.is_some(),
        Err(_) => false,
    }
}

pub async fn list_members(
    state: &SharedState,
    organization_id: &str,
) -> Result<Vec<OrganizationMemberRow>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT u.id, u.email, u.name, tm.role_id, r.name as role_name, tm.joined_at, u.status_id = 4, u.totp_enabled != 0
         FROM organization_members tm
         JOIN users u ON u.id = tm.user_id
         JOIN roles r ON r.id = tm.role_id
         WHERE tm.organization_id = ?1
         ORDER BY tm.joined_at ASC",
    )
    .bind(organization_id)
    .fetch_all(&conn)
    .await
    .map_err(|e| e.to_string())?;

    let mut members = Vec::new();
    for row in &rows {
        members.push(OrganizationMemberRow {
            user_id: row.try_get(0).unwrap(),
            email: row.try_get(1).unwrap(),
            name: row.try_get(2).unwrap(),
            role_id: row.try_get(3).unwrap(),
            role_name: row.try_get(4).unwrap(),
            joined_at: row.try_get(5).unwrap(),
            disabled: row.try_get(6).unwrap(),
            two_factor_enabled: row.try_get(7).unwrap(),
        });
    }
    Ok(members)
}

pub async fn list_memberships_for_user(
    state: &SharedState,
    user_id: &str,
) -> Result<Vec<UserMembershipRow>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT tm.organization_id, tm.role_id, r.name as role_name, tm.joined_at
         FROM organization_members tm
         JOIN roles r ON r.id = tm.role_id
         WHERE tm.user_id = ?1",
    )
    .bind(user_id)
    .fetch_all(&conn)
    .await
    .map_err(|e| e.to_string())?;

    let mut memberships = Vec::new();
    for row in &rows {
        memberships.push(UserMembershipRow {
            organization_id: row.try_get(0).unwrap(),
            role_id: row.try_get(1).unwrap(),
            role_name: row.try_get(2).unwrap(),
            joined_at: row.try_get(3).unwrap(),
        });
    }
    Ok(memberships)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_member_list_says_who_has_a_second_factor() {
        let (state, path) = crate::testing::test_state().await;
        sqlx::query("UPDATE users SET totp_enabled = 1 WHERE id = 'editor'")
            .execute(&state.db)
            .await
            .unwrap();

        let members = list_members(&state, "organization-a").await.unwrap();
        let flag = |id: &str| {
            members
                .iter()
                .find(|member| member.user_id == id)
                .unwrap_or_else(|| panic!("{id} is a member of organization-a"))
                .two_factor_enabled
        };
        assert!(flag("editor"));
        assert!(!flag("viewer"));

        crate::testing::cleanup_database(&path);
    }
}
