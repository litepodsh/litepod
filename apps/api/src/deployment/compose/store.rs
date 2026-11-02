use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    deployment::runtime_endpoints::{REMOTE_PORT_MAX, REMOTE_PORT_MIN},
    git::repositories::db,
    state::state::SharedState,
};

fn default_protocol() -> String {
    "tcp".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PortMapping {
    pub service: String,
    pub host_port: u16,
    pub container_port: u16,
    /// `"tcp"` or `"udp"`. Defaults to `"tcp"` so a `port-overrides.yml` written
    /// before this field existed still parses.
    #[serde(default = "default_protocol")]
    pub protocol: String,
}

pub struct ComposeFileStore {
    data_dir: PathBuf,
    application_id: String,
}

impl ComposeFileStore {
    pub async fn list_host_ports(state: &SharedState) -> Result<HashSet<u16>, String> {
        let data_dir = &state.data_dir;
        let compose_dir = data_dir.join("compose");
        if !compose_dir.exists() {
            return Ok(HashSet::new());
        }
        // A compose dir whose application was deleted before `deployment::cleanup`
        // also removed it (crash mid-delete, or an older build predating that
        // fix) must not go on reserving its port forever - cross-check against
        // live applications and skip anything orphaned.
        let live_ids: HashSet<String> = sqlx::query_scalar("SELECT id FROM applications")
            .fetch_all(&db::get_conn(state).await)
            .await
            .map_err(|e| format!("Failed to list applications: {e}"))?
            .into_iter()
            .collect();
        let mut entries = tokio::fs::read_dir(&compose_dir)
            .await
            .map_err(|e| format!("Failed to list Compose directories: {e}"))?;
        let mut ports = HashSet::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| format!("Failed to read Compose directory entry: {e}"))?
        {
            if !entry.file_type().await.map_err(|e| e.to_string())?.is_dir() {
                continue;
            }
            let application_id = entry.file_name().to_string_lossy().into_owned();
            if !live_ids.contains(&application_id) {
                continue;
            }
            let store = Self::new(data_dir, &application_id);
            ports.extend(
                store
                    .read_port_mappings()
                    .await?
                    .into_iter()
                    .map(|mapping| mapping.host_port),
            );
        }
        Ok(ports)
    }
    pub fn new(data_dir: &Path, application_id: &str) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            application_id: application_id.to_string(),
        }
    }

    fn base_dir(&self) -> PathBuf {
        self.data_dir.join("compose").join(&self.application_id)
    }

    pub fn content_path(&self) -> PathBuf {
        self.base_dir().join("docker-compose.yml")
    }

    pub fn override_path(&self) -> PathBuf {
        self.base_dir().join("port-overrides.yml")
    }

    pub async fn read_content(&self) -> Result<String, String> {
        let path = self.content_path();
        if !path.exists() {
            return Ok(String::new());
        }
        tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| format!("Failed to read compose file: {e}"))
    }

    /// Refuses content whose service names would shadow a control-plane container
    /// on the shared network, and content whose mounts are host paths, binds or
    /// anonymous volumes (which rootless Podman cannot write and which would let a
    /// tenant reach the host's files or the Podman socket). This is the single
    /// chokepoint every writer goes through - the Compose editor, the assistant
    /// and all internal templates - so an unsafe file can never be persisted, let
    /// alone deployed.
    pub async fn write_content(&self, content: &str) -> Result<(), String> {
        let reserved = super::reserved_services_in_compose(content);
        if !reserved.is_empty() {
            return Err(super::reserved_service_message(&reserved));
        }
        let mounts = super::unnamed_mounts_in_compose(content);
        if !mounts.is_empty() {
            return Err(super::unnamed_mounts_message(&mounts));
        }
        let dir = self.base_dir();
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| format!("Failed to create compose directory: {e}"))?;
        tokio::fs::write(self.content_path(), content)
            .await
            .map_err(|e| format!("Failed to write compose file: {e}"))
    }

    pub async fn read_port_mappings(&self) -> Result<Vec<PortMapping>, String> {
        let path = self.override_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| format!("Failed to read port overrides: {e}"))?;
        if content.trim().is_empty() {
            return Ok(Vec::new());
        }
        let value: serde_yaml::Value = serde_yaml::from_str(&content)
            .map_err(|e| format!("Invalid port overrides file: {e}"))?;
        let mut mappings = Vec::new();
        if let Some(services) = value.get("services").and_then(|v| v.as_mapping()) {
            for (service_name, service_config) in services {
                let name = service_name.as_str().unwrap_or("").to_string();
                if let Some(ports) = service_config.get("ports").and_then(|v| v.as_sequence()) {
                    for port_entry in ports {
                        if let Some(port_str) = port_entry.as_str() {
                            if let Some((host, rest)) = port_str.split_once(':') {
                                let (container, protocol) = match rest.split_once('/') {
                                    Some((container, protocol)) => (container, protocol),
                                    None => (rest, "tcp"),
                                };
                                if let (Ok(hp), Ok(cp)) =
                                    (host.parse::<u16>(), container.parse::<u16>())
                                {
                                    mappings.push(PortMapping {
                                        service: name.clone(),
                                        host_port: hp,
                                        container_port: cp,
                                        protocol: protocol.to_ascii_lowercase(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(mappings)
    }

    pub async fn write_port_mappings(&self, mappings: &[PortMapping]) -> Result<(), String> {
        if mappings.is_empty() {
            let _ = tokio::fs::remove_file(self.override_path()).await;
            return Ok(());
        }
        for mapping in mappings {
            if mapping.protocol != "tcp" && mapping.protocol != "udp" {
                return Err(format!(
                    "Port mapping for service '{}' has an invalid protocol '{}'; use 'tcp' or 'udp'",
                    mapping.service, mapping.protocol
                ));
            }
            // 42000-42999 is reserved for litepod's own managed loopback endpoints
            // (`runtime_endpoints::ensure_remote`) - a user-chosen public port must
            // never collide with that range on a remote server.
            if (REMOTE_PORT_MIN..=REMOTE_PORT_MAX).contains(&(mapping.host_port as i64)) {
                return Err(format!(
                    "Host port {} is reserved for litepod's internal use ({}-{}); choose a different port",
                    mapping.host_port, REMOTE_PORT_MIN, REMOTE_PORT_MAX
                ));
            }
        }
        let mut yaml = serde_yaml::Mapping::new();
        let mut services_map = serde_yaml::Mapping::new();
        for mapping in mappings {
            let entry = services_map
                .entry(serde_yaml::Value::String(mapping.service.clone()))
                .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
            if let Some(service_map) = entry.as_mapping_mut() {
                let ports = service_map
                    .entry(serde_yaml::Value::String("ports".to_string()))
                    .or_insert_with(|| serde_yaml::Value::Sequence(Vec::new()));
                if let Some(port_list) = ports.as_sequence_mut() {
                    port_list.push(serde_yaml::Value::String(format!(
                        "{}:{}/{}",
                        mapping.host_port, mapping.container_port, mapping.protocol
                    )));
                }
            }
        }
        yaml.insert(
            serde_yaml::Value::String("services".to_string()),
            serde_yaml::Value::Mapping(services_map),
        );
        let dir = self.base_dir();
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| format!("Failed to create compose directory: {e}"))?;
        let content = serde_yaml::to_string(&yaml)
            .map_err(|e| format!("Failed to serialize port overrides: {e}"))?;
        tokio::fs::write(self.override_path(), content)
            .await
            .map_err(|e| format!("Failed to write port overrides: {e}"))
    }

    pub async fn remove_all(&self) -> Result<(), String> {
        let dir = self.base_dir();
        if dir.exists() {
            tokio::fs::remove_dir_all(&dir)
                .await
                .map_err(|e| format!("Failed to remove compose directory: {e}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trips_content_and_mappings() {
        let dir =
            std::env::temp_dir().join(format!("litepod-compose-store-{}", uuid::Uuid::now_v7()));
        let store = ComposeFileStore::new(&dir, "app-1");
        let content = "services:\n  web:\n    image: nginx\n";
        store.write_content(content).await.unwrap();
        assert_eq!(store.read_content().await.unwrap(), content);

        let mappings = vec![
            PortMapping {
                service: "web".into(),
                host_port: 8080,
                container_port: 80,
                protocol: "tcp".into(),
            },
            PortMapping {
                service: "web".into(),
                host_port: 33020,
                container_port: 33020,
                protocol: "udp".into(),
            },
        ];
        store.write_port_mappings(&mappings).await.unwrap();
        let mut read = store.read_port_mappings().await.unwrap();
        read.sort_by_key(|mapping| mapping.host_port);
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].service, "web");
        assert_eq!(read[0].host_port, 8080);
        assert_eq!(read[0].container_port, 80);
        assert_eq!(read[0].protocol, "tcp");
        assert_eq!(read[1].host_port, 33020);
        assert_eq!(read[1].protocol, "udp");

        assert!(
            store
                .write_port_mappings(&[PortMapping {
                    service: "web".into(),
                    host_port: 42_500,
                    container_port: 80,
                    protocol: "tcp".into(),
                }])
                .await
                .is_err(),
            "a host port inside litepod's reserved remote-endpoint range must be rejected"
        );

        store.remove_all().await.unwrap();
        assert!(!store.content_path().parent().unwrap().exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
