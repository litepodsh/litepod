use sqlx::Row;

use crate::state::state::SharedState;

#[derive(Debug, Clone)]
pub struct ProjectRow {
    pub id: String,
    pub organization_id: String,
    pub name: String,
    pub description: String,
    pub icon_url: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct EnvironmentRow {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub description: String,
    pub is_production: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct ApplicationRow {
    pub id: String,
    pub environment_id: String,
    pub name: String,
    pub description: String,
    pub template_kind: Option<String>,
    pub catalog_template_id: Option<String>,
    pub catalog_icon_url: Option<String>,
    pub icon_url: Option<String>,
    pub server_id: Option<String>,
    pub server_group_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

const PROJECT_COLUMNS: &str =
    "id, organization_id, name, description, created_at, updated_at, icon_url";
const QUALIFIED_ENVIRONMENT_COLUMNS: &str =
    "e.id, e.project_id, e.name, e.description, e.is_production, e.created_at, e.updated_at";
const QUALIFIED_APPLICATION_COLUMNS: &str = "a.id, a.environment_id, a.name, a.description, a.template_kind, a.catalog_template_id, a.catalog_icon_url, a.server_id, a.created_at, a.updated_at, a.icon_url, a.server_group_id";

fn project_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<ProjectRow, String> {
    Ok(ProjectRow {
        id: row.try_get(0).map_err(|error| error.to_string())?,
        organization_id: row.try_get(1).map_err(|error| error.to_string())?,
        name: row.try_get(2).map_err(|error| error.to_string())?,
        description: row.try_get(3).map_err(|error| error.to_string())?,
        created_at: row.try_get(4).map_err(|error| error.to_string())?,
        updated_at: row.try_get(5).map_err(|error| error.to_string())?,
        icon_url: row.try_get(6).map_err(|error| error.to_string())?,
    })
}

fn environment_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<EnvironmentRow, String> {
    Ok(EnvironmentRow {
        id: row.try_get(0).map_err(|error| error.to_string())?,
        project_id: row.try_get(1).map_err(|error| error.to_string())?,
        name: row.try_get(2).map_err(|error| error.to_string())?,
        description: row.try_get(3).map_err(|error| error.to_string())?,
        is_production: row
            .try_get::<i64, _>(4)
            .map_err(|error| error.to_string())?
            != 0,
        created_at: row.try_get(5).map_err(|error| error.to_string())?,
        updated_at: row.try_get(6).map_err(|error| error.to_string())?,
    })
}

fn application_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<ApplicationRow, String> {
    Ok(ApplicationRow {
        id: row.try_get(0).map_err(|error| error.to_string())?,
        environment_id: row.try_get(1).map_err(|error| error.to_string())?,
        name: row.try_get(2).map_err(|error| error.to_string())?,
        description: row.try_get(3).map_err(|error| error.to_string())?,
        template_kind: row.try_get(4).map_err(|error| error.to_string())?,
        catalog_template_id: row.try_get(5).map_err(|error| error.to_string())?,
        catalog_icon_url: row.try_get(6).map_err(|error| error.to_string())?,
        server_id: row.try_get(7).map_err(|error| error.to_string())?,
        created_at: row.try_get(8).map_err(|error| error.to_string())?,
        updated_at: row.try_get(9).map_err(|error| error.to_string())?,
        icon_url: row.try_get(10).map_err(|error| error.to_string())?,
        server_group_id: row.try_get(11).map_err(|error| error.to_string())?,
    })
}

/// Called both standalone (with the pool) and inside a transaction when a
/// project and its production environment are created together, so it takes
/// a generic executor rather than a concrete connection type.
pub async fn insert_project<'e, E>(
    executor: E,
    id: &str,
    organization_id: &str,
    name: &str,
    description: &str,
    timestamp: i64,
) -> Result<(), String>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "INSERT INTO projects (id, organization_id, name, description, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
    )
    .bind(id)
    .bind(organization_id)
    .bind(name)
    .bind(description)
    .bind(timestamp)
    .execute(executor)
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}

/// Called both standalone (creating a project's initial environment) and
/// inside a transaction (creating a project together with its production
/// environment), so it takes a generic executor.
pub async fn insert_environment<'e, E>(
    executor: E,
    id: &str,
    project_id: &str,
    name: &str,
    description: &str,
    is_production: bool,
    timestamp: i64,
) -> Result<(), String>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    sqlx::query(
        "INSERT INTO environments (id, project_id, name, description, is_production, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
    )
    .bind(id)
    .bind(project_id)
    .bind(name)
    .bind(description)
    .bind(i64::from(is_production))
    .bind(timestamp)
    .execute(executor)
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}

pub async fn insert_application(
    conn: &sqlx::SqlitePool,
    id: &str,
    environment_id: &str,
    name: &str,
    description: &str,
    template_kind: Option<&str>,
    server_id: Option<&str>,
    server_group_id: Option<&str>,
    timestamp: i64,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO applications (id, environment_id, name, description, template_kind, server_id, server_group_id, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
    )
    .bind(id)
    .bind(environment_id)
    .bind(name)
    .bind(description)
    .bind(template_kind)
    .bind(server_id)
    .bind(server_group_id)
    .bind(timestamp)
    .execute(conn)
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}

pub async fn set_catalog_metadata(
    conn: &sqlx::SqlitePool,
    application_id: &str,
    catalog_template_id: &str,
    catalog_icon_url: Option<&str>,
    catalog_primary_port: Option<u16>,
    timestamp: i64,
) -> Result<(), String> {
    sqlx::query(
        "UPDATE applications SET catalog_template_id = ?1, catalog_icon_url = ?2, catalog_primary_port = ?3, updated_at = ?4 WHERE id = ?5",
    )
    .bind(catalog_template_id)
    .bind(catalog_icon_url)
    .bind(catalog_primary_port.map(i64::from))
    .bind(timestamp)
    .bind(application_id)
    .execute(conn)
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}

pub async fn get_catalog_primary_port(
    state: &SharedState,
    application_id: &str,
) -> Result<Option<u16>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query("SELECT catalog_primary_port FROM applications WHERE id = ?1")
        .bind(application_id)
        .fetch_optional(&conn)
        .await
        .map_err(|error| error.to_string())?;
    let Some(row) = row else {
        return Ok(None);
    };
    let port: Option<i64> = row.try_get(0).map_err(|error| error.to_string())?;
    Ok(port
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| *value > 0))
}

/// Stores every `[[ports]]` entry and the URL-related flags of the catalog
/// template an application came from, for `{{url:PORT}}` env markers.
pub async fn set_catalog_ports(
    conn: &sqlx::SqlitePool,
    application_id: &str,
    catalog_ports: &[crate::deployment::compose::domain_markers::CatalogPort],
    requires_https: bool,
    url_persisted: bool,
) -> Result<(), String> {
    let ports = serde_json::to_string(catalog_ports).map_err(|error| error.to_string())?;
    sqlx::query(
        "UPDATE applications SET catalog_ports = ?1, catalog_requires_https = ?2, catalog_url_persisted = ?3 WHERE id = ?4",
    )
    .bind(ports)
    .bind(requires_https as i64)
    .bind(url_persisted as i64)
    .bind(application_id)
    .execute(conn)
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}

/// The template's declared ports. Apps created before `catalog_ports` existed
/// only know their primary port.
pub async fn get_catalog_ports(
    state: &SharedState,
    application_id: &str,
) -> Result<Vec<crate::deployment::compose::domain_markers::CatalogPort>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row =
        sqlx::query("SELECT catalog_ports, catalog_primary_port FROM applications WHERE id = ?1")
            .bind(application_id)
            .fetch_optional(&conn)
            .await
            .map_err(|error| error.to_string())?;
    let Some(row) = row else {
        return Ok(Vec::new());
    };
    let ports: Option<String> = row.try_get(0).map_err(|error| error.to_string())?;
    if let Some(ports) = ports {
        return serde_json::from_str(&ports).map_err(|error| error.to_string());
    }
    let primary: Option<i64> = row.try_get(1).map_err(|error| error.to_string())?;
    Ok(primary
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port > 0)
        .map(
            |container| crate::deployment::compose::domain_markers::CatalogPort {
                container,
                service: None,
                primary: true,
            },
        )
        .into_iter()
        .collect())
}

/// `(requires_https, url_persisted)` from the app's catalog template.
pub async fn get_catalog_flags(
    state: &SharedState,
    application_id: &str,
) -> Result<(bool, bool), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(
        "SELECT catalog_requires_https, catalog_url_persisted FROM applications WHERE id = ?1",
    )
    .bind(application_id)
    .fetch_optional(&conn)
    .await
    .map_err(|error| error.to_string())?;
    let Some(row) = row else {
        return Ok((false, false));
    };
    let https: i64 = row.try_get(0).map_err(|error| error.to_string())?;
    let persisted: i64 = row.try_get(1).map_err(|error| error.to_string())?;
    Ok((https != 0, persisted != 0))
}

pub async fn list_projects(
    state: &SharedState,
    organization_id: &str,
) -> Result<Vec<ProjectRow>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {PROJECT_COLUMNS} FROM projects WHERE organization_id = ?1 ORDER BY updated_at DESC"
    ))
    .bind(organization_id)
    .fetch_all(&conn)
    .await
    .map_err(|error| error.to_string())?;
    let mut projects = Vec::new();
    for row in &rows {
        projects.push(project_from_row(row)?);
    }
    Ok(projects)
}

pub async fn get_project(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
) -> Result<Option<ProjectRow>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(&format!(
        "SELECT {PROJECT_COLUMNS} FROM projects WHERE id = ?1 AND organization_id = ?2"
    ))
    .bind(project_id)
    .bind(organization_id)
    .fetch_optional(&conn)
    .await
    .map_err(|error| error.to_string())?;
    row.as_ref().map(project_from_row).transpose()
}

pub async fn list_environments(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
) -> Result<Vec<EnvironmentRow>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {QUALIFIED_ENVIRONMENT_COLUMNS} FROM environments e INNER JOIN projects p ON p.id = e.project_id WHERE e.project_id = ?1 AND p.organization_id = ?2 ORDER BY e.is_production DESC, e.created_at ASC"
    ))
    .bind(project_id)
    .bind(organization_id)
    .fetch_all(&conn)
    .await
    .map_err(|error| error.to_string())?;
    let mut environments = Vec::new();
    for row in &rows {
        environments.push(environment_from_row(row)?);
    }
    Ok(environments)
}

pub async fn get_environment(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
) -> Result<Option<EnvironmentRow>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(&format!(
        "SELECT {QUALIFIED_ENVIRONMENT_COLUMNS} FROM environments e INNER JOIN projects p ON p.id = e.project_id WHERE e.id = ?1 AND e.project_id = ?2 AND p.organization_id = ?3"
    ))
    .bind(environment_id)
    .bind(project_id)
    .bind(organization_id)
    .fetch_optional(&conn)
    .await
    .map_err(|error| error.to_string())?;
    row.as_ref().map(environment_from_row).transpose()
}

pub async fn list_applications(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
) -> Result<Vec<ApplicationRow>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(&format!(
        "SELECT {QUALIFIED_APPLICATION_COLUMNS} FROM applications a INNER JOIN environments e ON e.id = a.environment_id INNER JOIN projects p ON p.id = e.project_id WHERE a.environment_id = ?1 AND e.project_id = ?2 AND p.organization_id = ?3 AND a.maintenance_for IS NULL ORDER BY a.created_at ASC"
    ))
    .bind(environment_id)
    .bind(project_id)
    .bind(organization_id)
    .fetch_all(&conn)
    .await
    .map_err(|error| error.to_string())?;
    let mut applications = Vec::new();
    for row in &rows {
        applications.push(application_from_row(row)?);
    }
    Ok(applications)
}

pub async fn get_application(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
) -> Result<Option<ApplicationRow>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let row = sqlx::query(&format!(
        "SELECT {QUALIFIED_APPLICATION_COLUMNS} FROM applications a INNER JOIN environments e ON e.id = a.environment_id INNER JOIN projects p ON p.id = e.project_id WHERE a.id = ?1 AND a.environment_id = ?2 AND e.project_id = ?3 AND p.organization_id = ?4"
    ))
    .bind(application_id)
    .bind(environment_id)
    .bind(project_id)
    .bind(organization_id)
    .fetch_optional(&conn)
    .await
    .map_err(|error| error.to_string())?;
    row.as_ref().map(application_from_row).transpose()
}

pub async fn update_project(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    name: &str,
    description: &str,
    icon_url: Option<&str>,
    updated_at: i64,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE projects SET name = ?1, description = ?2, icon_url = ?3, updated_at = ?4 WHERE id = ?5 AND organization_id = ?6",
    )
    .bind(name)
    .bind(description)
    .bind(icon_url)
    .bind(updated_at)
    .bind(project_id)
    .bind(organization_id)
    .execute(&conn)
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}

pub async fn update_environment(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    name: &str,
    description: &str,
    updated_at: i64,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE environments SET name = ?1, description = ?2, updated_at = ?3 WHERE id = ?4 AND project_id = ?5 AND EXISTS (SELECT 1 FROM projects WHERE id = ?5 AND organization_id = ?6)",
    )
    .bind(name)
    .bind(description)
    .bind(updated_at)
    .bind(environment_id)
    .bind(project_id)
    .bind(organization_id)
    .execute(&conn)
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}

pub async fn update_application(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
    name: &str,
    description: &str,
    icon_url: Option<&str>,
    server_id: Option<&str>,
    server_group_id: Option<&str>,
    updated_at: i64,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "UPDATE applications SET name = ?1, description = ?2, icon_url = ?3, server_id = ?4, server_group_id = ?5, updated_at = ?6 WHERE id = ?7 AND environment_id = ?8 AND EXISTS (SELECT 1 FROM environments e INNER JOIN projects p ON p.id = e.project_id WHERE e.id = ?8 AND e.project_id = ?9 AND p.organization_id = ?10)",
    )
    .bind(name)
    .bind(description)
    .bind(icon_url)
    .bind(server_id)
    .bind(server_group_id)
    .bind(updated_at)
    .bind(application_id)
    .bind(environment_id)
    .bind(project_id)
    .bind(organization_id)
    .execute(&conn)
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}

pub async fn list_project_application_ids(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
) -> Result<Vec<String>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT a.id FROM applications a INNER JOIN environments e ON e.id = a.environment_id INNER JOIN projects p ON p.id = e.project_id WHERE p.id = ?1 AND p.organization_id = ?2",
    )
    .bind(project_id)
    .bind(organization_id)
    .fetch_all(&conn)
    .await
    .map_err(|error| error.to_string())?;
    collect_ids(&rows)
}

pub async fn list_environment_application_ids(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
) -> Result<Vec<String>, String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT a.id FROM applications a INNER JOIN environments e ON e.id = a.environment_id INNER JOIN projects p ON p.id = e.project_id WHERE e.id = ?1 AND e.project_id = ?2 AND p.organization_id = ?3",
    )
    .bind(environment_id)
    .bind(project_id)
    .bind(organization_id)
    .fetch_all(&conn)
    .await
    .map_err(|error| error.to_string())?;
    collect_ids(&rows)
}

fn collect_ids(rows: &[sqlx::sqlite::SqliteRow]) -> Result<Vec<String>, String> {
    let mut ids = Vec::new();
    for row in rows {
        ids.push(
            row.try_get::<String, _>(0)
                .map_err(|error| error.to_string())?,
        );
    }
    Ok(ids)
}

pub async fn delete_project(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query("DELETE FROM projects WHERE id = ?1 AND organization_id = ?2")
        .bind(project_id)
        .bind(organization_id)
        .execute(&conn)
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub async fn delete_environment(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "DELETE FROM environments WHERE id = ?1 AND project_id = ?2 AND EXISTS (SELECT 1 FROM projects WHERE id = ?2 AND organization_id = ?3)",
    )
    .bind(environment_id)
    .bind(project_id)
    .bind(organization_id)
    .execute(&conn)
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}

pub async fn delete_application(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    application_id: &str,
) -> Result<(), String> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "DELETE FROM applications WHERE id = ?1 AND environment_id = ?2 AND EXISTS (SELECT 1 FROM environments e INNER JOIN projects p ON p.id = e.project_id WHERE e.id = ?2 AND e.project_id = ?3 AND p.organization_id = ?4)",
    )
    .bind(application_id)
    .bind(environment_id)
    .bind(project_id)
    .bind(organization_id)
    .execute(&conn)
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}
