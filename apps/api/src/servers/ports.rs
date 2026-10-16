//! Read-only aggregation of every host port an application on a server has
//! published, from every source litepod knows about. Nothing here computes a
//! new port or writes anything - it only reads what deploy-time code already
//! produces, so a collision between two apps on the same server is visible
//! before it turns into a Podman "address already in use" failure.

use serde::Serialize;

use crate::{
    deployment::{
        compose::{catalog::catalog_from_yaml, store::ComposeFileStore},
        runtime_endpoints,
    },
    dtos::response::AppError,
    servers::repository,
    state::state::SharedState,
};

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ServerPortDto {
    pub application_id: String,
    pub application_name: String,
    pub service: Option<String>,
    pub host_ip: Option<String>,
    /// A single port ("8080") or a range ("9000-9002"), never a number - so a
    /// range doesn't need a second, rarely-used field just for its case.
    pub host_port: String,
    pub container_port: String,
    pub protocol: String,
    /// "compose" (the app's own `ports:`), "port-override" (the Ports-section
    /// `port-overrides.yml`), or "managed-endpoint" (litepod's own loopback
    /// slot for an HTTP domain).
    pub source: String,
}

pub async fn list_for_server(
    state: &SharedState,
    server_id: &str,
) -> Result<Vec<ServerPortDto>, AppError> {
    let applications = repository::assigned_applications(state, server_id).await?;
    let mut ports = Vec::new();

    for application in &applications {
        let store = ComposeFileStore::new(&state.data_dir, &application.id);

        let content = store.read_content().await.unwrap_or_default();
        if !content.trim().is_empty() {
            match catalog_from_yaml(&content, false) {
                Ok(catalog) => {
                    for service in &catalog.services {
                        for exposure in &service.direct_exposures {
                            ports.push(ServerPortDto {
                                application_id: application.id.clone(),
                                application_name: application.name.clone(),
                                service: Some(exposure.service.clone()),
                                host_ip: exposure.host_ip.clone(),
                                host_port: exposure.published.display(),
                                container_port: exposure.target.display(),
                                protocol: exposure.protocol.clone(),
                                source: "compose".into(),
                            });
                        }
                    }
                }
                // A broken compose file shouldn't 500 the whole server's view -
                // every other application's ports still matter.
                Err(error) => tracing::warn!(
                    application_id = application.id,
                    "skipping compose ports for server ports view: {error}"
                ),
            }
        }

        match store.read_port_mappings().await {
            Ok(mappings) => {
                for mapping in mappings {
                    ports.push(ServerPortDto {
                        application_id: application.id.clone(),
                        application_name: application.name.clone(),
                        service: Some(mapping.service),
                        host_ip: None,
                        host_port: mapping.host_port.to_string(),
                        container_port: mapping.container_port.to_string(),
                        protocol: mapping.protocol,
                        source: "port-override".into(),
                    });
                }
            }
            Err(error) => tracing::warn!(
                application_id = application.id,
                "skipping port overrides for server ports view: {error}"
            ),
        }

        if let Some(endpoint) = runtime_endpoints::get(state, &application.id).await? {
            ports.push(ServerPortDto {
                application_id: application.id.clone(),
                application_name: application.name.clone(),
                service: None,
                host_ip: Some("127.0.0.1".into()),
                host_port: endpoint.host_port.to_string(),
                container_port: endpoint.container_port.to_string(),
                protocol: "tcp".into(),
                source: "managed-endpoint".into(),
            });
        }
    }

    Ok(ports)
}
