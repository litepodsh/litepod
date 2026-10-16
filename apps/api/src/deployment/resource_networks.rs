//! Per-resource Podman networks for the internal database templates.
//!
//! Every internal database resource (`postgres`, `mariadb`, ...) owns a network
//! named `<application_id>_db`. Only the applications the user explicitly links
//! join it, so a database stops being reachable by everything that happens to
//! share the host-wide `litepod-network`.
//!
//! The network is *not* written into the template's stored Compose file. It is
//! layered on at deploy time through a generated override, exactly like
//! [`crate::deployment::managed_volumes::compose_override`], so resources that
//! already exist gain their network without their file being rewritten.

use std::ffi::OsString;

use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    dtos::{
        deployment::{LinkedApplicationDto, ResourceNetworkLinkDto, ResourceNetworksDto},
        response::AppError,
    },
    git::repositories::db,
    servers::runtime::ServerRuntime,
    state::state::SharedState,
};

/// `template_kind` values that own a resource network. These are the internal
/// database templates in `crate::templates::internal_templates`.
pub const RESOURCE_TEMPLATE_KINDS: [&str; 6] = [
    "postgres",
    "mariadb",
    "keydb",
    "dragonflydb",
    "sqlserver",
    "mongodb",
];

/// The physical Podman network a database resource owns.
pub fn network_name(resource_application_id: &str) -> String {
    format!("{resource_application_id}_db")
}

/// Compose's logical key for that network. Application ids carry hyphens, which
/// read fine in YAML but make for a noisy key, and the prefix keeps it clear of
/// `litepod_proxy` and the generated `private` network.
pub fn network_key(resource_application_id: &str) -> String {
    let sanitized = resource_application_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("litepod_res_{sanitized}")
}

async fn template_kind(
    state: &SharedState,
    application_id: &str,
) -> Result<Option<String>, AppError> {
    let conn = db::get_conn(state).await;
    let row = sqlx::query("SELECT template_kind FROM applications WHERE id = ?1")
        .bind(application_id)
        .fetch_optional(&conn)
        .await?;
    match row {
        Some(row) => Ok(row.try_get(0)?),
        None => Ok(None),
    }
}

/// True when this application is an internal database resource, and therefore
/// publishes a network of its own.
pub async fn is_resource(state: &SharedState, application_id: &str) -> Result<bool, AppError> {
    Ok(template_kind(state, application_id)
        .await?
        .is_some_and(|kind| RESOURCE_TEMPLATE_KINDS.contains(&kind.as_str())))
}

/// Resource application ids this application is linked to, oldest link first.
async fn linked_resources(
    state: &SharedState,
    application_id: &str,
) -> Result<Vec<String>, AppError> {
    let conn = db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT resource_application_id FROM application_network_links WHERE application_id = ?1 ORDER BY created_at, id",
    )
    .bind(application_id)
    .fetch_all(&conn)
    .await?;
    let mut ids = Vec::with_capacity(rows.len());
    for row in &rows {
        ids.push(row.try_get(0)?);
    }
    Ok(ids)
}

/// Every resource network this application's containers must be able to join:
/// its own when it is a database resource, plus one per link it holds.
pub async fn networks_for(
    state: &SharedState,
    application_id: &str,
) -> Result<Vec<String>, AppError> {
    let mut ids = Vec::new();
    if is_resource(state, application_id).await? {
        ids.push(application_id.to_string());
    }
    ids.extend(linked_resources(state, application_id).await?);
    Ok(ids.iter().map(|id| network_name(id)).collect())
}

/// Builds the Compose override that attaches `services` to this application's
/// resource networks. Returns an empty string when there is nothing to attach,
/// so the caller can delete a stale override file the same way the managed
/// volume path does.
pub async fn compose_override(
    state: &SharedState,
    application_id: &str,
    services: &[String],
) -> Result<String, AppError> {
    let mut resource_ids = Vec::new();
    if is_resource(state, application_id).await? {
        resource_ids.push(application_id.to_string());
    }
    resource_ids.extend(linked_resources(state, application_id).await?);
    Ok(render_override(&resource_ids, services))
}

/// The YAML half of [`compose_override`], split out so it can be tested without
/// a database.
fn render_override(resource_ids: &[String], services: &[String]) -> String {
    if resource_ids.is_empty() || services.is_empty() {
        return String::new();
    }
    let mut networks = serde_yaml::Mapping::new();
    let mut service_networks = serde_yaml::Mapping::new();
    for resource_id in resource_ids {
        let key = serde_yaml::Value::String(network_key(resource_id));
        if networks.contains_key(&key) {
            continue;
        }
        service_networks.insert(
            key.clone(),
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
        );
        networks.insert(
            key,
            serde_yaml::Value::Mapping(serde_yaml::Mapping::from_iter([
                (
                    serde_yaml::Value::String("external".into()),
                    serde_yaml::Value::Bool(true),
                ),
                (
                    serde_yaml::Value::String("name".into()),
                    serde_yaml::Value::String(network_name(resource_id)),
                ),
            ])),
        );
    }
    let mut services_map = serde_yaml::Mapping::new();
    for service in services {
        services_map.insert(
            serde_yaml::Value::String(service.clone()),
            serde_yaml::Value::Mapping(serde_yaml::Mapping::from_iter([(
                serde_yaml::Value::String("networks".into()),
                serde_yaml::Value::Mapping(service_networks.clone()),
            )])),
        );
    }
    let root = serde_yaml::Mapping::from_iter([
        (
            serde_yaml::Value::String("services".into()),
            serde_yaml::Value::Mapping(services_map),
        ),
        (
            serde_yaml::Value::String("networks".into()),
            serde_yaml::Value::Mapping(networks),
        ),
    ]);
    serde_yaml::to_string(&serde_yaml::Value::Mapping(root)).unwrap_or_default()
}

/// Moves a database resource that was generated before resource networks onto
/// its own network: drops `litepod_proxy` from every service and from the root,
/// and declares the resource network in their place.
///
/// Returns `None` when the file already looks like that, so a resource is
/// rewritten exactly once. Comments are not preserved, which is fine: these
/// files are generated by the internal templates and carry none.
pub fn migrate_resource_compose(
    content: &str,
    application_id: &str,
) -> Result<Option<String>, String> {
    let mut document: serde_yaml::Value =
        serde_yaml::from_str(content).map_err(|error| format!("Invalid Compose file: {error}"))?;
    let proxy = serde_yaml::Value::String("litepod_proxy".into());
    let key = serde_yaml::Value::String(network_key(application_id));
    let networks_key = serde_yaml::Value::String("networks".into());
    let mut changed = false;

    let services = document
        .get_mut("services")
        .and_then(serde_yaml::Value::as_mapping_mut)
        .ok_or_else(|| "Invalid Compose file: expected a services mapping".to_string())?;
    for (_, config) in services.iter_mut() {
        let Some(config) = config.as_mapping_mut() else {
            continue;
        };
        // A service pinned to a host or container namespace has no networks of
        // its own to rewrite.
        if config.contains_key(serde_yaml::Value::String("network_mode".into())) {
            continue;
        }
        let entry = config
            .entry(networks_key.clone())
            .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
        // The generated templates always spell this as a mapping; a sequence can
        // only come from a hand edit, so normalize it rather than give up.
        if let Some(items) = entry.as_sequence() {
            let mapping = items
                .iter()
                .filter_map(serde_yaml::Value::as_str)
                .map(|name| {
                    (
                        serde_yaml::Value::String(name.to_owned()),
                        serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
                    )
                })
                .collect::<serde_yaml::Mapping>();
            *entry = serde_yaml::Value::Mapping(mapping);
            changed = true;
        }
        let Some(mapping) = entry.as_mapping_mut() else {
            continue;
        };
        if mapping.remove(&proxy).is_some() {
            changed = true;
        }
        if !mapping.contains_key(&key) {
            mapping.insert(
                key.clone(),
                serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
            );
            changed = true;
        }
    }

    let root = document
        .as_mapping_mut()
        .ok_or_else(|| "Invalid Compose file: expected a mapping".to_string())?;
    let networks = root
        .entry(networks_key)
        .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
        .as_mapping_mut()
        .ok_or_else(|| "Invalid Compose file: top-level networks must be a mapping".to_string())?;
    if networks.remove(&proxy).is_some() {
        changed = true;
    }
    let definition = serde_yaml::Value::Mapping(serde_yaml::Mapping::from_iter([
        (
            serde_yaml::Value::String("external".into()),
            serde_yaml::Value::Bool(true),
        ),
        (
            serde_yaml::Value::String("name".into()),
            serde_yaml::Value::String(network_name(application_id)),
        ),
    ]));
    if networks.get(&key) != Some(&definition) {
        networks.insert(key, definition);
        changed = true;
    }

    if !changed {
        return Ok(None);
    }
    serde_yaml::to_string(&document)
        .map(Some)
        .map_err(|error| format!("Could not rewrite the Compose file networks: {error}"))
}

/// Creates `names` on the server that will run the Compose project. Both sides
/// declare these networks `external: true`, so `podman compose up` fails if the
/// network is missing. `--ignore` makes this a no-op when it already exists,
/// same as the host bootstrap in `crate::servers::setup`.
pub async fn ensure_networks(runtime: &ServerRuntime, names: &[String]) -> Result<(), AppError> {
    for name in names {
        runtime
            .podman(&[
                OsString::from("network"),
                OsString::from("create"),
                OsString::from("--ignore"),
                OsString::from(name),
            ])
            .await?;
    }
    Ok(())
}

/// Resolves this application's resource networks and creates them on the server
/// that is about to run it. The single entry point for the deploy paths that
/// start a container directly, without Compose.
pub async fn ensure_for_deploy(
    state: &SharedState,
    runtime: &ServerRuntime,
    application_id: &str,
) -> Result<Vec<String>, AppError> {
    let names = networks_for(state, application_id).await?;
    ensure_networks(runtime, &names).await?;
    Ok(names)
}

/// Tears down a resource's own network. Best effort: a network still holding a
/// linked container, or one that was never created, must not fail the delete.
pub async fn remove_network(runtime: &ServerRuntime, resource_application_id: &str) {
    let _ = runtime
        .podman(&[
            OsString::from("network"),
            OsString::from("rm"),
            OsString::from("--force"),
            OsString::from(network_name(resource_application_id)),
        ])
        .await;
}

/// Database resources this application can be linked to: same environment, same
/// server. A Podman network is local to one host, so a resource on another
/// server could never be joined.
pub async fn list_linkable(
    state: &SharedState,
    application_id: &str,
) -> Result<Vec<ResourceNetworkLinkDto>, AppError> {
    let conn = db::get_conn(state).await;
    let placeholders = RESOURCE_TEMPLATE_KINDS
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT resource.id, resource.name, resource.template_kind, \
                link.id IS NOT NULL AS linked \
         FROM applications resource \
         INNER JOIN applications consumer ON consumer.id = ?1 \
         LEFT JOIN application_network_links link \
           ON link.application_id = consumer.id AND link.resource_application_id = resource.id \
         WHERE resource.environment_id = consumer.environment_id \
           AND resource.server_id = consumer.server_id \
           AND resource.id <> consumer.id \
           AND resource.template_kind IN ({placeholders}) \
         ORDER BY resource.name COLLATE NOCASE, resource.id"
    );
    let mut query = sqlx::query(&sql).bind(application_id);
    for kind in RESOURCE_TEMPLATE_KINDS {
        query = query.bind(kind);
    }
    let rows = query.fetch_all(&conn).await?;
    let mut links = Vec::with_capacity(rows.len());
    for row in &rows {
        let resource_application_id: String = row.try_get(0)?;
        let linked: i64 = row.try_get(3)?;
        links.push(ResourceNetworkLinkDto {
            network_name: network_name(&resource_application_id),
            resource_application_id,
            name: row.try_get(1)?,
            template_kind: row.try_get(2)?,
            linked: linked != 0,
        });
    }
    Ok(links)
}

/// Applications that have joined this resource's own network. Empty for an
/// application that is not a database resource, and — for one that is — the
/// signal that nothing can reach it.
pub async fn list_consumers(
    state: &SharedState,
    resource_application_id: &str,
) -> Result<Vec<LinkedApplicationDto>, AppError> {
    let conn = db::get_conn(state).await;
    let rows = sqlx::query(
        "SELECT consumer.id, consumer.name \
         FROM application_network_links link \
         INNER JOIN applications consumer ON consumer.id = link.application_id \
         WHERE link.resource_application_id = ?1 \
         ORDER BY consumer.name COLLATE NOCASE, consumer.id",
    )
    .bind(resource_application_id)
    .fetch_all(&conn)
    .await?;
    let mut consumers = Vec::with_capacity(rows.len());
    for row in &rows {
        consumers.push(LinkedApplicationDto {
            application_id: row.try_get(0)?,
            name: row.try_get(1)?,
        });
    }
    Ok(consumers)
}

/// Everything the Networks tab shows about resource networks, in one round trip.
pub async fn overview(
    state: &SharedState,
    application_id: &str,
) -> Result<ResourceNetworksDto, AppError> {
    Ok(ResourceNetworksDto {
        linkable: list_linkable(state, application_id).await?,
        consumers: if is_resource(state, application_id).await? {
            list_consumers(state, application_id).await?
        } else {
            Vec::new()
        },
    })
}

/// Links `application_id` to `resource_application_id`. Idempotent, and rejects
/// a target that is not a database resource this application could reach.
pub async fn attach(
    state: &SharedState,
    application_id: &str,
    resource_application_id: &str,
) -> Result<(), AppError> {
    let linkable = list_linkable(state, application_id).await?;
    let target = linkable
        .iter()
        .find(|link| link.resource_application_id == resource_application_id)
        .ok_or_else(|| {
            AppError::ValidationError(
                "That database resource is not in this application's environment and server".into(),
            )
        })?;
    if target.linked {
        return Ok(());
    }
    let conn = db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO application_network_links (id, application_id, resource_application_id, created_at) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(application_id)
    .bind(resource_application_id)
    .bind(Utc::now().timestamp_millis())
    .execute(&conn)
    .await?;
    Ok(())
}

/// Removes the link. Missing rows are not an error, so the UI switch can be
/// toggled off twice without failing.
pub async fn detach(
    state: &SharedState,
    application_id: &str,
    resource_application_id: &str,
) -> Result<(), AppError> {
    let conn = db::get_conn(state).await;
    sqlx::query(
        "DELETE FROM application_network_links WHERE application_id = ?1 AND resource_application_id = ?2",
    )
    .bind(application_id)
    .bind(resource_application_id)
    .execute(&conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> serde_yaml::Value {
        serde_yaml::from_str(yaml).expect("override is valid YAML")
    }

    #[test]
    fn nothing_to_attach_renders_nothing() {
        assert!(render_override(&[], &["web".into()]).is_empty());
        assert!(render_override(&["db-1234".into()], &[]).is_empty());
    }

    #[test]
    fn a_resource_gets_its_own_network() {
        let value = parse(&render_override(
            &["postgres-ab12".into()],
            &["postgres".into()],
        ));
        assert_eq!(
            value["networks"]["litepod_res_postgres_ab12"]["name"].as_str(),
            Some("postgres-ab12_db")
        );
        assert_eq!(
            value["networks"]["litepod_res_postgres_ab12"]["external"].as_bool(),
            Some(true)
        );
        assert!(
            value["services"]["postgres"]["networks"]
                .as_mapping()
                .expect("service networks are a mapping")
                .contains_key(serde_yaml::Value::String(
                    "litepod_res_postgres_ab12".into()
                ))
        );
    }

    #[test]
    fn every_service_joins_every_linked_network() {
        let value = parse(&render_override(
            &["postgres-ab12".into(), "keydb-cd34".into()],
            &["web".into(), "worker".into()],
        ));
        for service in ["web", "worker"] {
            let networks = value["services"][service]["networks"]
                .as_mapping()
                .expect("service networks are a mapping");
            assert_eq!(networks.len(), 2, "{service} joins both resource networks");
        }
        assert_eq!(
            value["networks"]["litepod_res_keydb_cd34"]["name"].as_str(),
            Some("keydb-cd34_db")
        );
    }

    #[test]
    fn a_repeated_resource_is_declared_once() {
        let value = parse(&render_override(
            &["postgres-ab12".into(), "postgres-ab12".into()],
            &["web".into()],
        ));
        assert_eq!(
            value["networks"]
                .as_mapping()
                .expect("networks are a mapping")
                .len(),
            1
        );
    }

    #[test]
    fn an_old_resource_moves_off_the_shared_network() {
        let content = "services:\n  mariadb:\n    image: mariadb:11.8\n    networks:\n      litepod_proxy: {}\nvolumes:\n  data: {}\nnetworks:\n  litepod_proxy:\n    external: true\n    name: litepod-network\n";
        let migrated = migrate_resource_compose(content, "mariadb1-ab12")
            .expect("valid Compose")
            .expect("the file had to change");
        assert!(!migrated.contains("litepod_proxy"));
        assert!(!migrated.contains("litepod-network"));
        let value: serde_yaml::Value = serde_yaml::from_str(&migrated).unwrap();
        assert_eq!(
            value["networks"]["litepod_res_mariadb1_ab12"]["name"].as_str(),
            Some("mariadb1-ab12_db")
        );
        assert!(
            value["services"]["mariadb"]["networks"]
                .as_mapping()
                .expect("service networks are a mapping")
                .contains_key(serde_yaml::Value::String(
                    "litepod_res_mariadb1_ab12".into()
                ))
        );
        assert_eq!(
            value["services"]["mariadb"]["image"].as_str(),
            Some("mariadb:11.8")
        );
        // Second pass has nothing left to do.
        assert!(
            migrate_resource_compose(&migrated, "mariadb1-ab12")
                .expect("valid Compose")
                .is_none()
        );
    }

    #[test]
    fn a_hand_written_network_list_is_rewritten_too() {
        let content = "services:\n  mariadb:\n    image: mariadb:11.8\n    networks: [litepod_proxy]\nnetworks:\n  litepod_proxy:\n    external: true\n    name: litepod-network\n";
        let migrated = migrate_resource_compose(content, "mariadb1-ab12")
            .expect("valid Compose")
            .expect("the file had to change");
        assert!(!migrated.contains("litepod_proxy"));
    }

    #[test]
    fn the_logical_key_survives_an_id_with_separators() {
        assert_eq!(network_key("My.App-1_2"), "litepod_res_my_app_1_2");
        assert_eq!(network_name("my-app-1"), "my-app-1_db");
    }
}
