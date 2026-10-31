use sqlx::Row;

use crate::state::state::SharedState;

pub struct OrganizationRow {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub icon: String,
    pub icon_bg: String,
    pub plan: String,
    pub is_root: bool,
    pub audit_logs_enabled: bool,
    pub audit_logs_retention_days: i64,
    /// The assistant's kill switch. Off until someone turns it on.
    pub assistant_enabled: bool,
    /// The MCP server's kill switch. Off until someone turns it on.
    pub mcp_enabled: bool,
    /// Every member must have two-factor authentication to act in the
    /// organization. Off until an admin turns it on.
    pub require_two_factor: bool,
    #[allow(dead_code)] // surfaced in audit reports; not read by current consumers.
    pub created_by: String,
    pub created_at: String,
}

pub async fn create_organization(
    state: &SharedState,
    id: &str,
    name: &str,
    slug: &str,
    description: &str,
    icon: &str,
    icon_bg: &str,
    plan: &str,
    created_by: &str,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    create_organization_on(
        &conn,
        id,
        name,
        slug,
        description,
        icon,
        icon_bg,
        plan,
        created_by,
    )
    .await
}

/// Called both standalone (with the pool) and inside a transaction (creating
/// an organization together with its founding membership), so it takes a
/// generic executor.
pub async fn create_organization_on<'e, E>(
    executor: E,
    id: &str,
    name: &str,
    slug: &str,
    description: &str,
    icon: &str,
    icon_bg: &str,
    plan: &str,
    created_by: &str,
) -> Result<(), String>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "INSERT INTO organizations (id, name, slug, description, icon, icon_bg, plan, created_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(id)
    .bind(name)
    .bind(slug)
    .bind(description)
    .bind(icon)
    .bind(icon_bg)
    .bind(plan)
    .bind(created_by)
    .execute(executor)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Only ever called inside a transaction (as a pre-insert count guard), so it
/// takes a generic executor rather than a concrete connection type.
pub async fn count_created_on<'e, E>(executor: E, user_id: &str) -> Result<i64, String>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let row = sqlx::query("SELECT COUNT(*) FROM organizations WHERE created_by = ?1")
        .bind(user_id)
        .fetch_one(executor)
        .await
        .map_err(|e| e.to_string())?;
    row.try_get(0).map_err(|e| e.to_string())
}

pub async fn create_root_organization(
    state: &SharedState,
    id: &str,
    name: &str,
    slug: &str,
    description: &str,
    icon: &str,
    plan: &str,
    created_by: &str,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO organizations (id, name, slug, description, icon, plan, is_root, created_by) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)",
    )
    .bind(id)
    .bind(name)
    .bind(slug)
    .bind(description)
    .bind(icon)
    .bind(plan)
    .bind(created_by)
    .execute(&conn)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn row_to_organization(row: &sqlx::sqlite::SqliteRow) -> OrganizationRow {
    OrganizationRow {
        id: row.try_get(0).unwrap(),
        name: row.try_get(1).unwrap(),
        slug: row.try_get(2).unwrap(),
        description: row.try_get(3).unwrap(),
        icon: row.try_get(4).unwrap_or_default(),
        icon_bg: row.try_get(5).unwrap_or_default(),
        plan: row.try_get(6).unwrap_or_default(),
        is_root: row.try_get::<i64, _>(7).unwrap_or(0) != 0,
        audit_logs_enabled: row.try_get::<i64, _>(8).unwrap_or(1) != 0,
        audit_logs_retention_days: row.try_get(9).unwrap_or(90),
        assistant_enabled: row.try_get::<i64, _>(10).unwrap_or(0) != 0,
        created_by: row.try_get(11).unwrap(),
        created_at: row.try_get(12).unwrap(),
        mcp_enabled: row.try_get::<i64, _>(13).unwrap_or(0) != 0,
        require_two_factor: row.try_get::<i64, _>(14).unwrap_or(0) != 0,
    }
}

const ORGANIZATION_COLUMNS: &str = "id, name, slug, description, icon, icon_bg, plan, is_root, audit_logs_enabled, audit_logs_retention_days, assistant_enabled, created_by, created_at, mcp_enabled, require_two_factor";

pub async fn get_organization_by_id(state: &SharedState, id: &str) -> Option<OrganizationRow> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(&format!(
        "SELECT {ORGANIZATION_COLUMNS} FROM organizations WHERE id = ?1"
    ))
    .bind(id)
    .fetch_optional(&conn)
    .await
    .ok()??;

    Some(row_to_organization(&row))
}

pub async fn get_organization_by_slug(state: &SharedState, slug: &str) -> Option<OrganizationRow> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(&format!(
        "SELECT {ORGANIZATION_COLUMNS} FROM organizations WHERE slug = ?1"
    ))
    .bind(slug)
    .fetch_optional(&conn)
    .await
    .ok()??;

    Some(row_to_organization(&row))
}

pub async fn list_organizations_for_user(
    state: &SharedState,
    user_id: &str,
) -> Result<Vec<OrganizationRow>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT t.id, t.name, t.slug, t.description, t.icon, t.icon_bg, t.plan, t.is_root, t.audit_logs_enabled, t.audit_logs_retention_days, t.assistant_enabled, t.created_by, t.created_at, t.mcp_enabled, t.require_two_factor
         FROM organizations t
         INNER JOIN organization_members tm ON tm.organization_id = t.id
         WHERE tm.user_id = ?1
         ORDER BY t.is_root DESC, t.created_at DESC",
    )
    .bind(user_id)
    .fetch_all(&conn)
    .await
    .map_err(|e| e.to_string())?;

    let mut organizations = Vec::new();
    for row in &rows {
        organizations.push(row_to_organization(row));
    }
    Ok(organizations)
}

pub async fn list_all_organizations(state: &SharedState) -> Result<Vec<OrganizationRow>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {ORGANIZATION_COLUMNS} FROM organizations ORDER BY is_root DESC, name COLLATE NOCASE ASC"
    ))
    .fetch_all(&conn)
    .await
    .map_err(|e| e.to_string())?;

    let mut organizations = Vec::new();
    for row in &rows {
        organizations.push(row_to_organization(row));
    }
    Ok(organizations)
}

pub async fn list_admin_organizations_for_user(
    state: &SharedState,
    user_id: &str,
) -> Result<Vec<OrganizationRow>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT t.id, t.name, t.slug, t.description, t.icon, t.icon_bg, t.plan, t.is_root, t.audit_logs_enabled, t.audit_logs_retention_days, t.assistant_enabled, t.created_by, t.created_at, t.mcp_enabled, t.require_two_factor
         FROM organizations t
         JOIN organization_members tm ON tm.organization_id = t.id
         WHERE tm.user_id = ?1 AND tm.role_id = 1
         ORDER BY t.name COLLATE NOCASE ASC",
    )
    .bind(user_id)
    .fetch_all(&conn)
    .await
    .map_err(|e| e.to_string())?;
    let mut organizations = Vec::new();
    for row in &rows {
        organizations.push(row_to_organization(row));
    }
    Ok(organizations)
}

pub async fn update_organization(
    state: &SharedState,
    id: &str,
    name: Option<&str>,
    description: Option<&str>,
    icon: Option<&str>,
    icon_bg: Option<&str>,
    plan: Option<&str>,
    audit_logs_enabled: Option<bool>,
    audit_logs_retention_days: Option<i64>,
    assistant_enabled: Option<bool>,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    if let Some(name) = name {
        sqlx::query("UPDATE organizations SET name = ?1 WHERE id = ?2")
            .bind(name)
            .bind(id)
            .execute(&conn)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(description) = description {
        sqlx::query("UPDATE organizations SET description = ?1 WHERE id = ?2")
            .bind(description)
            .bind(id)
            .execute(&conn)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(icon) = icon {
        sqlx::query("UPDATE organizations SET icon = ?1 WHERE id = ?2")
            .bind(icon)
            .bind(id)
            .execute(&conn)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(icon_bg) = icon_bg {
        sqlx::query("UPDATE organizations SET icon_bg = ?1 WHERE id = ?2")
            .bind(icon_bg)
            .bind(id)
            .execute(&conn)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(plan) = plan {
        sqlx::query("UPDATE organizations SET plan = ?1 WHERE id = ?2")
            .bind(plan)
            .bind(id)
            .execute(&conn)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(audit_logs_enabled) = audit_logs_enabled {
        sqlx::query("UPDATE organizations SET audit_logs_enabled = ?1 WHERE id = ?2")
            .bind(i64::from(audit_logs_enabled))
            .bind(id)
            .execute(&conn)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(audit_logs_retention_days) = audit_logs_retention_days {
        sqlx::query("UPDATE organizations SET audit_logs_retention_days = ?1 WHERE id = ?2")
            .bind(audit_logs_retention_days)
            .bind(id)
            .execute(&conn)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(assistant_enabled) = assistant_enabled {
        sqlx::query("UPDATE organizations SET assistant_enabled = ?1 WHERE id = ?2")
            .bind(i64::from(assistant_enabled))
            .bind(id)
            .execute(&conn)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Kept out of `update_organization`'s positional list on purpose: one more
/// `Option<bool>` there is one more argument a caller can put in the wrong slot.
pub async fn set_mcp_enabled(state: &SharedState, id: &str, enabled: bool) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE organizations SET mcp_enabled = ?1 WHERE id = ?2")
        .bind(i64::from(enabled))
        .bind(id)
        .execute(&conn)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Separate from `update_organization` for the same reason as `set_mcp_enabled`.
pub async fn set_require_two_factor(
    state: &SharedState,
    id: &str,
    required: bool,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("UPDATE organizations SET require_two_factor = ?1 WHERE id = ?2")
        .bind(i64::from(required))
        .bind(id)
        .execute(&conn)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Whether `id` requires two-factor authentication. One column read, so the
/// organization extractor can ask it on every request.
pub async fn requires_two_factor(state: &SharedState, id: &str) -> Result<bool, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let required: Option<i64> =
        sqlx::query_scalar("SELECT require_two_factor FROM organizations WHERE id = ?1")
            .bind(id)
            .fetch_optional(&conn)
            .await
            .map_err(|e| e.to_string())?;
    Ok(required.unwrap_or(0) != 0)
}

pub async fn delete_organization(state: &SharedState, id: &str) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("DELETE FROM organizations WHERE id = ?1")
        .bind(id)
        .execute(&conn)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod mcp_toggle_tests {
    use super::*;

    #[tokio::test]
    async fn mcp_is_off_until_someone_turns_it_on() {
        let (state, path) = crate::testing::test_state().await;

        let before = get_organization_by_id(&state, "organization-a")
            .await
            .unwrap();
        assert!(
            !before.mcp_enabled,
            "an organization must not start with MCP on"
        );

        set_mcp_enabled(&state, "organization-a", true)
            .await
            .unwrap();
        let after = get_organization_by_id(&state, "organization-a")
            .await
            .unwrap();
        assert!(after.mcp_enabled);

        // Only the organization asked about changes.
        let other = get_organization_by_id(&state, "organization-b")
            .await
            .unwrap();
        assert!(!other.mcp_enabled);

        crate::testing::cleanup_database(&path);
    }
}

#[cfg(test)]
mod require_two_factor_tests {
    use super::*;

    #[tokio::test]
    async fn two_factor_is_not_required_until_an_admin_asks() {
        let (state, path) = crate::testing::test_state().await;

        let before = get_organization_by_id(&state, "organization-a")
            .await
            .unwrap();
        assert!(
            !before.require_two_factor,
            "an organization must not start by requiring 2FA"
        );
        assert!(!requires_two_factor(&state, "organization-a").await.unwrap());

        set_require_two_factor(&state, "organization-a", true)
            .await
            .unwrap();
        let after = get_organization_by_id(&state, "organization-a")
            .await
            .unwrap();
        assert!(after.require_two_factor);
        assert!(requires_two_factor(&state, "organization-a").await.unwrap());

        // Only the organization asked about changes.
        assert!(!requires_two_factor(&state, "organization-b").await.unwrap());
        // An organization that does not exist requires nothing.
        assert!(!requires_two_factor(&state, "missing").await.unwrap());

        crate::testing::cleanup_database(&path);
    }
}
