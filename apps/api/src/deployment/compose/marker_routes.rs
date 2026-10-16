//! DB-facing half of domain markers: loads env, catalog ports, routes and
//! domains, then defers to the pure `domain_markers`.

use crate::{
    deployment::{
        compose::{domain_markers, proxy_routes},
        decrypted_environment, domains,
    },
    dtos::{
        deployment::{DomainMarkerDto, DomainMarkersDto},
        response::AppError,
    },
    git::repositories::project,
    state::state::SharedState,
};

/// Creates a generated domain + proxy route for every `(service, port)` a
/// marker names that has no route yet. Existing routes are never touched, even
/// when the user has changed their domain.
pub async fn ensure_routes(
    state: &SharedState,
    application_id: &str,
    compose: &str,
) -> Result<Vec<String>, AppError> {
    let env = decrypted_environment(state, application_id, "runtime").await?;
    if !domain_markers::has_markers(&env) {
        return Ok(Vec::new());
    }
    let ports = project::get_catalog_ports(state, application_id)
        .await
        .map_err(AppError::DatabaseError)?;
    let required = domain_markers::required_routes(&env, &ports, compose)
        .map_err(AppError::ValidationError)?;
    let existing = proxy_routes::list(state, application_id).await?;
    // A pasted Compose app gets a generated default domain before any service
    // is picked for it. Route those unclaimed generated domains first (primary
    // first) instead of leaving them orphaned next to fresh ones. A custom
    // domain the user added is never taken over.
    let mut unrouted: Vec<_> = domains::list(state, application_id)
        .await?
        .into_iter()
        .filter(|domain| domain.is_local || domain.host.ends_with(".sslip.io"))
        .filter(|domain| !existing.iter().any(|route| route.domain_id == domain.id))
        .collect();
    unrouted.sort_by_key(|domain| (!domain.is_primary, domain.created_at));
    let mut unrouted = unrouted.into_iter();
    // The app's public service gets first pick, so a reused primary domain
    // lands on it rather than on whichever service sorts first.
    let public_service = crate::deployment::source::get(state, application_id)
        .await?
        .compose_public_service;
    let mut required: Vec<_> = required.into_iter().collect();
    required.sort_by_key(|target| Some(&target.service) != public_service.as_ref());
    let mut created = Vec::new();
    for target in required {
        if existing
            .iter()
            .any(|route| route.service == target.service && route.container_port == target.port)
        {
            continue;
        }
        let domain = match unrouted.next() {
            Some(domain) => domain,
            None => domains::generate_with(state, application_id, false).await?,
        };
        // `upsert`, not `save`: the deploy that is running pushes every route
        // after `up`, and `save` would flag the deployment stale mid-deploy.
        proxy_routes::upsert(
            state,
            application_id,
            &domain.id,
            &target.service,
            target.port,
        )
        .await?;
        created.push(domain.host);
    }
    Ok(created)
}

/// The runtime env with every domain marker replaced by its route's address.
pub async fn resolve_runtime_env(
    state: &SharedState,
    application_id: &str,
    compose: &str,
    env: Vec<(String, String)>,
) -> Result<Vec<(String, String)>, AppError> {
    if !domain_markers::has_markers(&env) {
        return Ok(env);
    }
    let ports = project::get_catalog_ports(state, application_id)
        .await
        .map_err(AppError::DatabaseError)?;
    let routes = proxy_routes::list(state, application_id).await?;
    let domains = domains::list(state, application_id).await?;
    domain_markers::resolve_env(
        env,
        &ports,
        compose,
        &domain_markers::addresses(&routes, &domains),
    )
    .map_err(AppError::ValidationError)
}

async fn stored_compose(state: &SharedState, application_id: &str) -> Result<String, AppError> {
    crate::deployment::compose::store::ComposeFileStore::new(&state.data_dir, application_id)
        .read_content()
        .await
        .map_err(AppError::InternalError)
}

/// Domain markers only make sense where litepod owns Compose routes. A marker
/// in a git/image app, or one naming a service the Compose file lacks, is
/// rejected on save so it can never reach a container literally.
pub async fn validate_env(
    state: &SharedState,
    application_id: &str,
    env: &[(String, String)],
) -> Result<(), AppError> {
    if !domain_markers::has_markers(env) {
        return Ok(());
    }
    if !domains::is_compose_application(state, application_id).await? {
        return Err(AppError::ValidationError(
            "Domain markers ({{url}}, {{domain}}) only work in Compose applications".into(),
        ));
    }
    let compose = stored_compose(state, application_id).await?;
    let ports = project::get_catalog_ports(state, application_id)
        .await
        .map_err(AppError::DatabaseError)?;
    domain_markers::required_routes(env, &ports, &compose)
        .map(|_| ())
        .map_err(AppError::ValidationError)
}

/// Whether a runtime env marker reads the route `service:port`, i.e. whether a
/// change to that route's domains changes a variable on the next deploy.
pub async fn references_route(
    state: &SharedState,
    application_id: &str,
    service: &str,
    port: u16,
) -> Result<bool, AppError> {
    let env = decrypted_environment(state, application_id, "runtime").await?;
    if !domain_markers::has_markers(&env) {
        return Ok(false);
    }
    let compose = stored_compose(state, application_id).await?;
    let ports = project::get_catalog_ports(state, application_id)
        .await
        .map_err(AppError::DatabaseError)?;
    Ok(domain_markers::required_routes(&env, &ports, &compose)
        .map(|routes| {
            routes
                .iter()
                .any(|route| route.service == service && route.port == port)
        })
        .unwrap_or(false))
}

/// Every domain marker in the runtime env with its current address, for the
/// env and domains tabs. Marker values are not secrets, but a value that merely
/// *looks* like a broken marker might be, so an unparsable value is never
/// echoed back.
pub async fn describe(
    state: &SharedState,
    application_id: &str,
) -> Result<DomainMarkersDto, AppError> {
    let (requires_https, url_persisted) = project::get_catalog_flags(state, application_id)
        .await
        .map_err(AppError::DatabaseError)?;
    let env = decrypted_environment(state, application_id, "runtime").await?;
    let mut markers = Vec::new();
    if domain_markers::has_markers(&env) {
        let compose = stored_compose(state, application_id)
            .await
            .unwrap_or_default();
        let ports = project::get_catalog_ports(state, application_id)
            .await
            .map_err(AppError::DatabaseError)?;
        let addresses = domain_markers::addresses(
            &proxy_routes::list(state, application_id).await?,
            &domains::list(state, application_id).await?,
        );
        for (name, value) in &env {
            let found = match domain_markers::markers_in(value) {
                Ok(found) => found,
                Err(error) => {
                    markers.push(DomainMarkerDto {
                        name: name.clone(),
                        marker: "{{…}}".into(),
                        service: None,
                        port: None,
                        domain_id: None,
                        resolved: None,
                        error: Some(error),
                    });
                    continue;
                }
            };
            for marker in found {
                let mut dto = DomainMarkerDto {
                    name: name.clone(),
                    marker: marker.raw.clone(),
                    service: None,
                    port: None,
                    domain_id: None,
                    resolved: None,
                    error: None,
                };
                match domain_markers::route_for(&marker.target, &ports, &compose) {
                    Ok(route) => {
                        match addresses.get(&route) {
                            Some(address) => {
                                dto.domain_id = Some(address.domain_id.clone());
                                dto.resolved = Some(match marker.kind {
                                    domain_markers::MarkerKind::Url => address.url.clone(),
                                    domain_markers::MarkerKind::Domain => address.host.clone(),
                                });
                            }
                            None => {
                                dto.error = Some(format!(
                                    "No domain for {}:{} yet; it is created on the next deploy",
                                    route.service, route.port
                                ));
                            }
                        }
                        dto.service = Some(route.service);
                        dto.port = Some(route.port);
                    }
                    Err(error) => dto.error = Some(error),
                }
                markers.push(dto);
            }
        }
    }
    Ok(DomainMarkersDto {
        requires_https,
        url_persisted,
        markers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deployment::compose::{domain_markers::CatalogPort, store::ComposeFileStore};

    const ENTE_COMPOSE: &str = "services:\n  ente-museum:\n    expose: [\"8080\"]\n  ente-web:\n    expose: [\"3000\", \"3002\"]\n";

    async fn marker_state() -> (SharedState, std::path::PathBuf, std::path::PathBuf) {
        let data_dir =
            std::env::temp_dir().join(format!("litepod-markers-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&data_dir).unwrap();
        let (state, path) =
            crate::testing::test_state_with_network("203.0.113.7", data_dir.clone()).await;
        (state, path, data_dir)
    }

    fn cleanup(path: &std::path::Path, data_dir: &std::path::Path) {
        crate::testing::cleanup_database(path);
        let _ = std::fs::remove_dir_all(data_dir);
    }

    /// A deployed-looking Compose app created from an Ente-like catalog
    /// template, with `env` as its runtime environment.
    async fn seed_marker_app(state: &SharedState, env: &str) -> String {
        let (_, _, app) = crate::testing::seed_application(state, "organization-a").await;
        // The local server: generating a domain on the seeded SSH server would
        // try to reach it.
        sqlx::query(
            "UPDATE applications SET deploy_type = 'compose', server_id = 'local' WHERE id = ?1",
        )
        .bind(&app)
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query("INSERT INTO application_deployments (application_id, status, container_name, hostname, updated_at) VALUES (?1, 'running', 'ente', '', 0)")
            .bind(&app)
            .execute(&state.db)
            .await
            .unwrap();
        ComposeFileStore::new(&state.data_dir, &app)
            .write_content(ENTE_COMPOSE)
            .await
            .unwrap();
        project::set_catalog_ports(
            &state.db,
            &app,
            &[
                CatalogPort {
                    container: 3000,
                    service: None,
                    primary: true,
                },
                CatalogPort {
                    container: 3002,
                    service: None,
                    primary: false,
                },
                CatalogPort {
                    container: 8080,
                    service: None,
                    primary: false,
                },
            ],
            true,
            false,
        )
        .await
        .unwrap();
        crate::deployment::save_environment(state, &app, "runtime", env)
            .await
            .unwrap();
        app
    }

    #[tokio::test]
    async fn ensure_routes_creates_one_domain_per_marker_port_and_resolves() {
        let (state, path, data_dir) = marker_state().await;
        let app = seed_marker_app(&state, "API={{url:8080}}\nALBUMS={{url:3002}}\n").await;

        let created = ensure_routes(&state, &app, ENTE_COMPOSE).await.unwrap();
        assert_eq!(created.len(), 2, "{created:?}");
        // Idempotent.
        assert!(
            ensure_routes(&state, &app, ENTE_COMPOSE)
                .await
                .unwrap()
                .is_empty()
        );

        let env = decrypted_environment(&state, &app, "runtime")
            .await
            .unwrap();
        let resolved = resolve_runtime_env(&state, &app, ENTE_COMPOSE, env)
            .await
            .unwrap();
        let api = &resolved.iter().find(|(name, _)| name == "API").unwrap().1;
        let albums = &resolved
            .iter()
            .find(|(name, _)| name == "ALBUMS")
            .unwrap()
            .1;
        for value in [api, albums] {
            assert!(
                value.starts_with("http://litepod-") && value.ends_with(".203.0.113.7.sslip.io"),
                "{value}"
            );
        }
        assert_ne!(api, albums);
        // The DB still holds the markers.
        let stored = decrypted_environment(&state, &app, "runtime")
            .await
            .unwrap();
        assert!(stored.iter().all(|(_, value)| value.starts_with("{{url:")));
        cleanup(&path, &data_dir);
    }

    #[tokio::test]
    async fn marker_free_env_creates_nothing() {
        let (state, path, data_dir) = marker_state().await;
        let app = seed_marker_app(&state, "PLAIN=1\n").await;
        assert!(
            ensure_routes(&state, &app, ENTE_COMPOSE)
                .await
                .unwrap()
                .is_empty()
        );
        cleanup(&path, &data_dir);
    }

    #[tokio::test]
    async fn validate_env_rejects_markers_outside_compose_and_bad_targets() {
        let (state, path, data_dir) = marker_state().await;
        // A plain (non-Compose) app: markers rejected, plain values fine.
        let (_, _, plain) = crate::testing::seed_application(&state, "organization-b").await;
        let marker = vec![("A".to_string(), "{{url:ente-web:3000}}".to_string())];
        assert!(validate_env(&state, &plain, &marker).await.is_err());
        assert!(
            validate_env(&state, &plain, &[("A".into(), "x".into())])
                .await
                .is_ok()
        );
        // A Compose app: valid targets pass, unknown services fail.
        let app = seed_marker_app(&state, "").await;
        assert!(validate_env(&state, &app, &marker).await.is_ok());
        let unknown = vec![("A".to_string(), "{{url:nope:3000}}".to_string())];
        assert!(validate_env(&state, &app, &unknown).await.is_err());
        cleanup(&path, &data_dir);
    }

    #[tokio::test]
    async fn deleting_a_marker_domain_marks_the_app_stale() {
        let (state, path, data_dir) = marker_state().await;
        let app = seed_marker_app(&state, "API={{url:8080}}\n").await;
        ensure_routes(&state, &app, ENTE_COMPOSE).await.unwrap();
        sqlx::query(
            "UPDATE application_deployments SET config_stale = 0 WHERE application_id = ?1",
        )
        .bind(&app)
        .execute(&state.db)
        .await
        .unwrap();
        let domain_id = proxy_routes::list(&state, &app)
            .await
            .unwrap()
            .into_iter()
            .find(|route| route.container_port == 8080)
            .unwrap()
            .domain_id;

        domains::delete(&state, &app, &domain_id).await.unwrap();

        let stale: i64 = sqlx::query_scalar(
            "SELECT config_stale FROM application_deployments WHERE application_id = ?1",
        )
        .bind(&app)
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(stale, 1);
        // The marker now points nowhere, and both describe and resolve say so.
        let described = describe(&state, &app).await.unwrap();
        assert!(described.markers[0].error.is_some());
        let env = decrypted_environment(&state, &app, "runtime")
            .await
            .unwrap();
        let error = resolve_runtime_env(&state, &app, ENTE_COMPOSE, env)
            .await
            .unwrap_err();
        assert!(
            matches!(&error, AppError::ValidationError(message) if message == "API references ente-museum:8080, which has no domain"),
            "{error:?}"
        );
        cleanup(&path, &data_dir);
    }

    #[tokio::test]
    async fn describe_reports_resolved_markers_and_flags() {
        let (state, path, data_dir) = marker_state().await;
        let app = seed_marker_app(
            &state,
            "API={{url:8080}}\nHOST={{domain:3002}}\nSECRET=hunter2\n",
        )
        .await;
        ensure_routes(&state, &app, ENTE_COMPOSE).await.unwrap();
        let described = describe(&state, &app).await.unwrap();
        assert!(described.requires_https);
        assert!(!described.url_persisted);
        assert_eq!(described.markers.len(), 2);
        let api = described.markers.iter().find(|m| m.name == "API").unwrap();
        assert_eq!(api.service.as_deref(), Some("ente-museum"));
        assert!(
            api.resolved
                .as_deref()
                .unwrap()
                .starts_with("http://litepod-")
        );
        assert!(api.domain_id.is_some());
        let host = described.markers.iter().find(|m| m.name == "HOST").unwrap();
        assert!(host.resolved.as_deref().unwrap().starts_with("litepod-"));
        cleanup(&path, &data_dir);
    }

    fn image_source() -> crate::dtos::deployment::UpdateSourceRequest {
        crate::dtos::deployment::UpdateSourceRequest {
            deploy_type: "image".into(),
            build_method: "docker".into(),
            git_provider_id: None,
            git_url: None,
            git_branch: None,
            auto_deploy: Some(false),
            watch_paths: None,
            dockerfile_path: None,
            compose_file_path: None,
            compose_public_service: None,
            container_port: Some(80),
            railpack_project_path: None,
            railpack_start_command: None,
            build_cpu_cores: None,
            build_memory_mib: None,
            build_swap_mib: None,
            image_url: Some("docker.io/library/nginx:latest".into()),
            registry_username: None,
            registry_password: None,
            restart_policy: None,
        }
    }

    #[tokio::test]
    async fn leaving_compose_with_markers_is_rejected() {
        let (state, path, data_dir) = marker_state().await;
        let app = seed_marker_app(&state, "API={{url:8080}}\n").await;
        let error = crate::deployment::source::save(&state, &app, image_source())
            .await
            .unwrap_err();
        assert!(
            matches!(&error, AppError::ValidationError(message) if message.contains("domain markers")),
            "{error:?}"
        );
        cleanup(&path, &data_dir);
    }

    #[tokio::test]
    async fn ensure_routes_reuses_an_unrouted_default_domain() {
        let (state, path, data_dir) = marker_state().await;
        let app = seed_marker_app(&state, "WEB={{url:ente-web:3000}}\n").await;
        let default = domains::generate(&state, &app).await.unwrap();

        ensure_routes(&state, &app, ENTE_COMPOSE).await.unwrap();

        let all = domains::list(&state, &app).await.unwrap();
        assert_eq!(all.len(), 1, "no extra domain when one is free");
        let routes = proxy_routes::list(&state, &app).await.unwrap();
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].domain_id, default.id);
        assert_eq!(routes[0].service, "ente-web");
        cleanup(&path, &data_dir);
    }

    #[tokio::test]
    async fn reused_primary_goes_to_the_public_service() {
        let (state, path, data_dir) = marker_state().await;
        let app = seed_marker_app(
            &state,
            "API={{url:ente-museum:8080}}\nWEB={{url:ente-web:3000}}\n",
        )
        .await;
        sqlx::query("UPDATE applications SET compose_public_service = 'ente-web' WHERE id = ?1")
            .bind(&app)
            .execute(&state.db)
            .await
            .unwrap();
        let default = domains::generate(&state, &app).await.unwrap();

        ensure_routes(&state, &app, ENTE_COMPOSE).await.unwrap();

        let routes = proxy_routes::list(&state, &app).await.unwrap();
        let primary_route = routes.iter().find(|r| r.domain_id == default.id).unwrap();
        assert_eq!(primary_route.service, "ente-web");
        assert_eq!(routes.len(), 2);
        cleanup(&path, &data_dir);
    }
}
