//! Bulk read for the organization-wide application lists (home, apps, domains,
//! analytics). Those lists used to fetch one deployment and one log-target
//! listing per application, which is a 35-request burst for a 31-app
//! organization — past the global rate limit, so rows silently came back empty.
//! Everything here is a fixed number of queries plus one container listing per
//! server, whatever the application count.

use std::collections::{HashMap, HashSet};

use sqlx::Row;

use crate::{
    deployment::{
        deployment_identity,
        domains::{domain_url, route_id},
        replicas::{
            STATUS_DEPLOYING, STATUS_FAILED, STATUS_NOT_DEPLOYED, STATUS_RUNNING, STATUS_STOPPED,
        },
    },
    dtos::{
        organization_apps::{
            AppContainersDto, AppSummaryApplicationDto, AppSummaryDomainDto, AppSummaryDto,
            AppSummaryEnvironmentDto, AppSummaryProjectDto,
        },
        response::AppError,
    },
    git::repositories::db::get_conn,
    handler::deployments::is_running_container,
    servers::runtime::ServerRuntime,
    state::state::SharedState,
};

fn db(error: sqlx::Error) -> AppError {
    AppError::DatabaseError(error.to_string())
}

fn read(row: &sqlx::sqlite::SqliteRow, index: usize) -> Result<String, AppError> {
    row.try_get(index)
        .map_err(|error| AppError::DatabaseError(error.to_string()))
}

fn read_opt(row: &sqlx::sqlite::SqliteRow, index: usize) -> Result<Option<String>, AppError> {
    row.try_get(index)
        .map_err(|error| AppError::DatabaseError(error.to_string()))
}

fn read_int(row: &sqlx::sqlite::SqliteRow, index: usize) -> Result<i64, AppError> {
    row.try_get(index)
        .map_err(|error| AppError::DatabaseError(error.to_string()))
}

fn read_opt_int(row: &sqlx::sqlite::SqliteRow, index: usize) -> Result<Option<i64>, AppError> {
    row.try_get(index)
        .map_err(|error| AppError::DatabaseError(error.to_string()))
}

/// One application with every column the summary needs, so the assembly below
/// runs without further lookups per row.
struct AppRow {
    application_id: String,
    project_id: String,
    project_name: String,
    environment_id: String,
    environment_name: String,
    is_production: bool,
    name: String,
    template_kind: Option<String>,
    catalog_icon_url: Option<String>,
    icon_url: Option<String>,
    created_at: i64,
    updated_at: i64,
    deploy_type: String,
    server_id: Option<String>,
    server_group_id: Option<String>,
    container_name: Option<String>,
    status: Option<String>,
    deployed_at: Option<i64>,
}

const APPS_SQL: &str = "SELECT p.id, p.name, e.id, e.name, e.is_production,
       a.id, a.name, a.template_kind, a.catalog_icon_url, a.icon_url,
       a.created_at, a.updated_at, a.deploy_type, a.server_id, a.server_group_id,
       d.container_name, d.status, d.deployed_at
FROM applications a
INNER JOIN environments e ON e.id = a.environment_id
INNER JOIN projects p ON p.id = e.project_id
LEFT JOIN application_deployments d ON d.application_id = a.id
WHERE p.organization_id = ?1 AND a.maintenance_for IS NULL
ORDER BY a.updated_at DESC";

const DOMAINS_SQL: &str = "SELECT d.application_id, d.id, d.host, d.port, d.https, d.is_primary
FROM application_domains d
INNER JOIN applications a ON a.id = d.application_id
INNER JOIN environments e ON e.id = a.environment_id
INNER JOIN projects p ON p.id = e.project_id
WHERE p.organization_id = ?1 AND a.maintenance_for IS NULL
ORDER BY d.created_at, d.id";

/// Certificate names for the same domains. The join is keyed on `route_id`
/// because that is what `certificates::load` resolves per domain one at a time.
const CERTIFICATES_SQL: &str = "SELECT dc.route_id, c.name
FROM domain_certificates dc
LEFT JOIN https_certificates c ON c.id = dc.certificate_id
WHERE dc.route_id IN (
    SELECT 'litepod-route-' || d.id
    FROM application_domains d
    INNER JOIN applications a ON a.id = d.application_id
    INNER JOIN environments e ON e.id = a.environment_id
    INNER JOIN projects p ON p.id = e.project_id
    WHERE p.organization_id = ?1 AND a.maintenance_for IS NULL
)";

const REPLICAS_SQL: &str = "SELECT r.application_id, r.server_id, r.status
FROM application_replicas r
INNER JOIN applications a ON a.id = r.application_id
INNER JOIN environments e ON e.id = a.environment_id
INNER JOIN projects p ON p.id = e.project_id
WHERE p.organization_id = ?1 AND a.maintenance_for IS NULL
ORDER BY r.application_id, r.server_id";

const GROUP_MEMBERS_SQL: &str = "SELECT DISTINCT m.group_id, m.server_id
FROM server_group_members m
WHERE m.group_id IN (
    SELECT DISTINCT a.server_group_id FROM applications a
    INNER JOIN environments e ON e.id = a.environment_id
    INNER JOIN projects p ON p.id = e.project_id
    WHERE p.organization_id = ?1 AND a.server_group_id IS NOT NULL AND a.maintenance_for IS NULL
)";

/// Every application of the organization, with its domains, deployment state
/// and container counts, in one call.
pub async fn list_organization_apps(
    state: &SharedState,
    organization_id: &str,
) -> Result<Vec<AppSummaryDto>, AppError> {
    let conn = get_conn(state).await;
    let rows = sqlx::query(APPS_SQL)
        .bind(organization_id)
        .fetch_all(&conn)
        .await
        .map_err(db)?;
    let apps: Vec<AppRow> = rows
        .iter()
        .map(|row| {
            Ok(AppRow {
                project_id: read(row, 0)?,
                project_name: read(row, 1)?,
                environment_id: read(row, 2)?,
                environment_name: read(row, 3)?,
                is_production: read_int(row, 4)? != 0,
                application_id: read(row, 5)?,
                name: read(row, 6)?,
                template_kind: read_opt(row, 7)?,
                catalog_icon_url: read_opt(row, 8)?,
                icon_url: read_opt(row, 9)?,
                created_at: read_int(row, 10)?,
                updated_at: read_int(row, 11)?,
                deploy_type: read(row, 12)?,
                server_id: read_opt(row, 13)?,
                server_group_id: read_opt(row, 14)?,
                container_name: read_opt(row, 15)?,
                status: read_opt(row, 16)?,
                deployed_at: read_opt_int(row, 17)?,
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;

    let domains = domains_by_application(&conn, organization_id).await?;
    let replicas = replicas_by_application(&conn, organization_id).await?;
    let group_members = group_members(&conn, organization_id).await?;
    let counts = container_counts(state, &apps, &replicas, &group_members).await;

    Ok(apps
        .into_iter()
        .map(|app| {
            let status = match replicas.status.get(&app.application_id) {
                Some(aggregate) => aggregate.clone(),
                None => app
                    .status
                    .clone()
                    .unwrap_or_else(|| STATUS_NOT_DEPLOYED.into()),
            };
            AppSummaryDto {
                application: AppSummaryApplicationDto {
                    id: app.application_id.clone(),
                    name: app.name.clone(),
                    template_kind: app.template_kind.clone(),
                    catalog_icon_url: app.catalog_icon_url.clone(),
                    icon_url: app.icon_url.clone(),
                    created_at: app.created_at,
                    updated_at: app.updated_at,
                },
                project: AppSummaryProjectDto {
                    id: app.project_id.clone(),
                    name: app.project_name.clone(),
                },
                environment: AppSummaryEnvironmentDto {
                    id: app.environment_id.clone(),
                    name: app.environment_name.clone(),
                    is_production: app.is_production,
                },
                status,
                deployed_at: app.deployed_at,
                domains: domains
                    .get(&app.application_id)
                    .cloned()
                    .unwrap_or_default(),
                containers: counts.get(&app.application_id).cloned().flatten(),
            }
        })
        .collect())
}

/// Domains per application, with the saved certificate name each one shows in
/// the lists. Built from two fixed queries; never one query per application.
async fn domains_by_application(
    conn: &sqlx::SqlitePool,
    organization_id: &str,
) -> Result<HashMap<String, Vec<AppSummaryDomainDto>>, AppError> {
    let rows = sqlx::query(DOMAINS_SQL)
        .bind(organization_id)
        .fetch_all(conn)
        .await
        .map_err(db)?;
    let certificates = sqlx::query(CERTIFICATES_SQL)
        .bind(organization_id)
        .fetch_all(conn)
        .await
        .map_err(db)?;
    let mut certificate_names: HashMap<String, Option<String>> = HashMap::new();
    for row in &certificates {
        certificate_names.insert(read(row, 0)?, read_opt(row, 1)?);
    }

    let mut by_application: HashMap<String, Vec<AppSummaryDomainDto>> = HashMap::new();
    for row in &rows {
        let application_id = read(row, 0)?;
        let id = read(row, 1)?;
        let host = read(row, 2)?;
        let port = read_int(row, 3)?;
        let https = read_int(row, 4)? != 0;
        let domain = AppSummaryDomainDto {
            certificate_name: certificate_names.get(&route_id(&id)).cloned().flatten(),
            url: domain_url(&host, port, https),
            id,
            host,
            port,
            https,
            is_primary: read_int(row, 5)? != 0,
        };
        by_application
            .entry(application_id)
            .or_default()
            .push(domain);
    }
    Ok(by_application)
}

/// Replica state per application: where it runs, plus the status aggregated
/// with the same precedence as `deployment::replicas::aggregate`. Applications
/// without replicas are absent from both maps.
struct ReplicasByApplication {
    servers: HashMap<String, Vec<String>>,
    status: HashMap<String, String>,
}

async fn replicas_by_application(
    conn: &sqlx::SqlitePool,
    organization_id: &str,
) -> Result<ReplicasByApplication, AppError> {
    let rows = sqlx::query(REPLICAS_SQL)
        .bind(organization_id)
        .fetch_all(conn)
        .await
        .map_err(db)?;
    let mut servers: HashMap<String, Vec<String>> = HashMap::new();
    let mut per_application: HashMap<String, Vec<String>> = HashMap::new();
    for row in &rows {
        let application_id = read(row, 0)?;
        servers
            .entry(application_id.clone())
            .or_default()
            .push(read(row, 1)?);
        per_application
            .entry(application_id)
            .or_default()
            .push(read(row, 2)?);
    }
    let status = per_application
        .into_iter()
        .map(|(application_id, statuses)| (application_id, aggregate_status(&statuses)))
        .collect();
    Ok(ReplicasByApplication { servers, status })
}

fn aggregate_status(statuses: &[String]) -> String {
    let has = |status: &str| statuses.iter().any(|value| value == status);
    if has(STATUS_RUNNING) {
        STATUS_RUNNING.into()
    } else if has(STATUS_FAILED) {
        STATUS_FAILED.into()
    } else if has(STATUS_DEPLOYING) {
        STATUS_DEPLOYING.into()
    } else if has(STATUS_STOPPED) {
        STATUS_STOPPED.into()
    } else {
        STATUS_NOT_DEPLOYED.into()
    }
}

async fn group_members(
    conn: &sqlx::SqlitePool,
    organization_id: &str,
) -> Result<HashMap<String, Vec<String>>, AppError> {
    let rows = sqlx::query(GROUP_MEMBERS_SQL)
        .bind(organization_id)
        .fetch_all(conn)
        .await
        .map_err(db)?;
    let mut members: HashMap<String, Vec<String>> = HashMap::new();
    for row in &rows {
        members
            .entry(read(row, 0)?)
            .or_default()
            .push(read(row, 1)?);
    }
    Ok(members)
}

/// `(running, total)` per application, `None` for an application one of its
/// servers could not answer for. `Some(0, 0)` is the honest "runs nothing"
/// case the lists show as `0/0`.
async fn container_counts(
    state: &SharedState,
    apps: &[AppRow],
    replicas: &ReplicasByApplication,
    group_members: &HashMap<String, Vec<String>>,
) -> HashMap<String, Option<AppContainersDto>> {
    let mut servers_by_application: HashMap<&str, Vec<String>> = HashMap::new();
    for app in apps {
        let servers = match replicas.servers.get(&app.application_id) {
            Some(servers) if !servers.is_empty() => servers.clone(),
            _ => match (&app.server_id, &app.server_group_id) {
                (Some(server_id), _) => vec![server_id.clone()],
                (None, Some(group_id)) => group_members.get(group_id).cloned().unwrap_or_default(),
                (None, None) => Vec::new(),
            },
        };
        servers_by_application.insert(app.application_id.as_str(), servers);
    }

    let mut listings: HashMap<String, Option<Vec<(String, bool)>>> = HashMap::new();
    for server_id in servers_by_application
        .values()
        .flatten()
        .collect::<HashSet<_>>()
    {
        listings.insert(
            server_id.clone(),
            list_server_containers(state, server_id).await,
        );
    }

    apps.iter()
        .map(|app| {
            let runtime_name = runtime_name(state, app);
            let servers = servers_by_application
                .get(app.application_id.as_str())
                .map(Vec::as_slice)
                .unwrap_or_default();
            let mut running = 0i64;
            let mut total = 0i64;
            let mut reachable = !servers.is_empty();
            for server_id in servers {
                match listings.get(server_id).and_then(Option::as_ref) {
                    Some(list) => {
                        for (name, is_up) in list {
                            if belongs_to(name, &runtime_name) {
                                total += 1;
                                if *is_up {
                                    running += 1;
                                }
                            }
                        }
                    }
                    None => reachable = false,
                }
            }
            let counts = reachable.then_some(AppContainersDto { running, total });
            (app.application_id.clone(), counts)
        })
        .collect()
}

/// Mirrors `deployment::rollout::active_runtime_name`: the name the last deploy
/// recorded, falling back to the derived identity when the row predates it.
fn runtime_name(state: &SharedState, app: &AppRow) -> String {
    let stored = app.container_name.clone().unwrap_or_default();
    if !stored.is_empty() && !(app.deploy_type == "compose" && stored.ends_with("-compose")) {
        return stored.strip_suffix("-public").unwrap_or(&stored).to_owned();
    }
    deployment_identity(
        &app.project_name,
        &app.environment_name,
        &app.name,
        &app.application_id,
        &state.public_ip,
    )
    .0
}

/// A container belongs to an application when it is named after its runtime
/// name, exactly (static deploys) or as the compose service prefix.
fn belongs_to(container_name: &str, runtime_name: &str) -> bool {
    container_name == runtime_name
        || container_name
            .strip_prefix(runtime_name)
            .is_some_and(|rest| rest.starts_with('-'))
}

/// One `podman ps` per server, shared by every application on it. `None` when
/// the server could not be reached — the lists show that as unknown, not as
/// "no containers".
async fn list_server_containers(
    state: &SharedState,
    server_id: &str,
) -> Option<Vec<(String, bool)>> {
    let runtime = ServerRuntime::for_server(state, server_id).await.ok()?;
    let output = runtime
        .podman(&[
            "ps".into(),
            "--all".into(),
            "--format".into(),
            "json".into(),
        ])
        .await
        .ok()?;
    parse_container_listing(&output.stdout)
}

/// Podman writes some `ps --format json` fields as a single string and others as
/// a one-element array depending on version, same as
/// `compose_standalone::json_string`.
fn parse_container_listing(stdout: &str) -> Option<Vec<(String, bool)>> {
    let value: serde_json::Value = serde_json::from_str(stdout).ok()?;
    let entries = value.as_array().cloned().unwrap_or_else(|| vec![value]);
    Some(
        entries
            .iter()
            .filter_map(|entry| {
                let name = crate::deployment::compose_standalone::json_string(
                    entry
                        .get("Names")
                        .or_else(|| entry.get("names"))
                        .or_else(|| entry.get("Name"))
                        .or_else(|| entry.get("name")),
                )?
                .to_string();
                let status = crate::deployment::compose_standalone::json_string(
                    entry
                        .get("State")
                        .or_else(|| entry.get("state"))
                        .or_else(|| entry.get("Status"))
                        .or_else(|| entry.get("status")),
                )
                .unwrap_or("unknown");
                Some((name, is_running_container(status)))
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An `offline` server answers the placement check but never reaches Podman,
    /// so container counts come back unknown without any network in tests.
    async fn seed_server(state: &SharedState, id: &str, organization_id: &str) {
        sqlx::query(
            "INSERT INTO servers (id, kind, name, organization_id, ingress_mode, state, ssh_host, ssh_port, ssh_user, created_at, updated_at) VALUES (?1, 'ssh', ?1, ?2, 'public_edge', 'offline', '198.51.100.1', 22, 'litepod', 1, 1)",
        )
        .bind(id)
        .bind(organization_id)
        .execute(&state.db)
        .await
        .unwrap();
    }

    async fn seed_application(
        state: &SharedState,
        application_id: &str,
        name: &str,
        organization_id: &str,
        server_id: &str,
    ) {
        let project_id = format!("{application_id}-project");
        let environment_id = format!("{application_id}-env");
        sqlx::query("INSERT INTO projects (id, organization_id, name, description, created_at, updated_at) VALUES (?1, ?2, ?1, '', 1, 1)")
            .bind(&project_id)
            .bind(organization_id)
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO environments (id, project_id, name, description, is_production, created_at, updated_at) VALUES (?1, ?2, 'prod', '', 1, 1, 1)")
            .bind(&environment_id)
            .bind(&project_id)
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO applications (id, environment_id, name, description, server_id, deploy_type, created_at, updated_at) VALUES (?1, ?2, ?3, '', ?4, 'compose', 1, 1)")
            .bind(application_id)
            .bind(&environment_id)
            .bind(name)
            .bind(server_id)
            .execute(&state.db)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn summary_carries_status_domains_and_deploy_time() {
        let (state, _path) = crate::testing::test_state().await;
        seed_server(&state, "srv-a", "organization-a").await;
        seed_application(&state, "app-a", "Ghost", "organization-a", "srv-a").await;
        sqlx::query("INSERT INTO application_deployments (application_id, status, container_name, hostname, updated_at, deployed_at) VALUES ('app-a', 'running', 'ghost-abc123', 'host', 1, 1792652044062)")
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO application_domains (id, application_id, host, port, https, is_primary, created_at, updated_at) VALUES ('dom-a', 'app-a', 'blog.example.com', 2368, 1, 1, 1, 1)")
            .execute(&state.db)
            .await
            .unwrap();

        let apps = list_organization_apps(&state, "organization-a")
            .await
            .unwrap();
        assert_eq!(apps.len(), 1);
        let app = &apps[0];
        assert_eq!(app.status, "running");
        assert_eq!(app.deployed_at, Some(1792652044062));
        assert_eq!(app.application.name, "Ghost");
        assert_eq!(app.domains.len(), 1);
        assert_eq!(app.domains[0].url, "https://blog.example.com");
        assert!(app.domains[0].is_primary);
        assert_eq!(app.domains[0].certificate_name, None);
        // The offline server cannot answer for its containers, and that must not
        // read as "runs nothing".
        assert!(app.containers.is_none());
    }

    #[tokio::test]
    async fn an_application_without_a_deployment_reads_as_not_deployed() {
        let (state, _path) = crate::testing::test_state().await;
        seed_server(&state, "srv-a", "organization-a").await;
        seed_application(&state, "app-a", "Loader", "organization-a", "srv-a").await;

        let apps = list_organization_apps(&state, "organization-a")
            .await
            .unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].status, "not_deployed");
        assert_eq!(apps[0].deployed_at, None);
        assert!(apps[0].domains.is_empty());
    }

    #[tokio::test]
    async fn an_application_of_another_organization_is_not_returned() {
        let (state, _path) = crate::testing::test_state().await;
        seed_server(&state, "srv-a", "organization-a").await;
        seed_application(&state, "app-a", "Ghost", "organization-a", "srv-a").await;
        seed_application(&state, "app-b", "Other", "organization-a", "srv-a").await;
        // Same server, different tenant: never listed for organization-a.
        sqlx::query(
            "UPDATE projects SET organization_id = 'organization-b' WHERE id = 'app-b-project'",
        )
        .execute(&state.db)
        .await
        .unwrap();

        let apps = list_organization_apps(&state, "organization-a")
            .await
            .unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].application.id, "app-a");
    }

    #[tokio::test]
    async fn a_domain_carries_its_saved_certificate_name() {
        let (state, _path) = crate::testing::test_state().await;
        seed_server(&state, "srv-a", "organization-a").await;
        seed_application(&state, "app-a", "Ghost", "organization-a", "srv-a").await;
        sqlx::query("INSERT INTO application_domains (id, application_id, host, port, https, is_primary, created_at, updated_at) VALUES ('dom-a', 'app-a', 'blog.example.com', 2368, 1, 1, 1, 1)")
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO https_certificates (id, name, certificate_pem, encrypted_private_key, created_at, updated_at) VALUES ('cert-a', 'Wildcard', 'pem', 'key', 1, 1)")
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO domain_certificates (route_id, provider, certificate_id) VALUES ('litepod-route-dom-a', 'custom', 'cert-a')")
            .execute(&state.db)
            .await
            .unwrap();

        let apps = list_organization_apps(&state, "organization-a")
            .await
            .unwrap();
        assert_eq!(
            apps[0].domains[0].certificate_name.as_deref(),
            Some("Wildcard")
        );
    }

    #[test]
    fn a_container_belongs_to_the_application_whose_name_it_carries() {
        assert!(belongs_to("ghost-abc123-95fdbc", "ghost-abc123-95fdbc"));
        assert!(belongs_to("ghost-abc123-95fdbc-db", "ghost-abc123-95fdbc"));
        // A sibling application whose runtime name extends this one is separate.
        assert!(!belongs_to(
            "ghost-abc123-95fdbcx-db",
            "ghost-abc123-95fdbc"
        ));
        assert!(!belongs_to("other-95fdbc", "ghost-abc123-95fdbc"));
    }

    #[test]
    fn a_running_replica_beats_a_failed_one() {
        let statuses = vec!["failed".to_string(), "running".to_string()];
        assert_eq!(aggregate_status(&statuses), STATUS_RUNNING);
        assert_eq!(aggregate_status(&["failed".to_string()]), STATUS_FAILED);
        assert_eq!(aggregate_status(&["stopped".to_string()]), STATUS_STOPPED);
        assert_eq!(
            aggregate_status(&["queued".to_string()]),
            STATUS_NOT_DEPLOYED
        );
    }

    #[test]
    fn podman_listing_accepts_both_field_shapes() {
        let listing = parse_container_listing(
            r#"[
                { "Names": ["ghost-abc123-95fdbc-ghost"], "State": "running" },
                { "Names": "ghost-abc123-95fdbc-db", "Status": "Exited (0) 2 days ago" },
                { "names": "loader-7f3fbf", "state": "up" }
            ]"#,
        )
        .unwrap();
        assert_eq!(
            listing,
            vec![
                ("ghost-abc123-95fdbc-ghost".to_string(), true),
                ("ghost-abc123-95fdbc-db".to_string(), false),
                ("loader-7f3fbf".to_string(), true),
            ]
        );
        assert_eq!(parse_container_listing("  "), None);
    }
}
