use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::Utc;
use rand::RngExt;
use serde::Deserialize;
use url::Url;

use crate::{
    cqrs::commands::project as project_cmd,
    deployment::{
        self,
        compose::{domain_markers, store::ComposeFileStore},
        source,
    },
    dtos::{
        deployment::UpdateSourceRequest,
        project::{ApplicationDto, CreateApplicationRequest},
        response::AppError,
    },
    git::repositories::project,
    state::state::SharedState,
};

const DEFAULT_CATALOG_ORIGIN: &str = "https://templates.litepod.sh";
const CATALOG_PATH: &str = "/api/v1/templates";
const MAX_BYTES: usize = 1024 * 1024;
/// Tells the catalog this litepod resolves `{{url}}` env markers; without it the
/// catalog hides templates that use them.
const CATALOG_FEATURES: &str = "domain-markers";

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogPort {
    pub container: u16,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub primary: bool,
    /// Compose service that owns the port, when several services expose it.
    #[serde(default)]
    pub service: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub struct CatalogIcon {
    pub path: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct CatalogFile {
    pub filename: String,
    pub content: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct CatalogTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub ports: Vec<CatalogPort>,
    pub icon: Option<CatalogIcon>,
    #[serde(default)]
    pub ui: Option<String>,
    pub website: Option<String>,
    pub docs: Option<String>,
    pub source: Option<String>,
    pub license: Option<String>,
    pub manifest: Option<CatalogFile>,
    pub compose: Option<CatalogFile>,
    pub env: Option<CatalogFile>,
    #[serde(default)]
    pub setup: Option<CatalogFile>,
    #[serde(default, rename = "requiresHttps")]
    pub requires_https: bool,
    #[serde(default, rename = "urlPersisted")]
    pub url_persisted: bool,
}
#[derive(Debug, Default, Deserialize)]
pub struct CatalogQuery {
    pub q: Option<String>,
    pub category: Option<String>,
    pub limit: Option<u8>,
}
#[derive(Deserialize)]
struct Envelope<T> {
    error: Option<String>,
    data: Option<T>,
}
#[derive(Clone)]
pub struct DecodedTemplate {
    pub template: CatalogTemplate,
    pub manifest: String,
    pub compose: String,
    pub environment: String,
    pub setup: String,
    pub icon_url: Option<String>,
}

impl CatalogQuery {
    fn append_to(&self, url: &mut reqwest::Url) -> Result<(), AppError> {
        if let Some(limit) = self.limit {
            if !(1..=100).contains(&limit) {
                return Err(AppError::ValidationError(
                    "limit must be between 1 and 100".into(),
                ));
            }
        }
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("features", CATALOG_FEATURES);
        if let Some(q) = self
            .q
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            pairs.append_pair("q", q);
        }
        if let Some(category) = self
            .category
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            pairs.append_pair("category", category);
        }
        if let Some(limit) = self.limit {
            pairs.append_pair("limit", &limit.to_string());
        }
        Ok(())
    }
}

fn catalog_origin() -> String {
    std::env::var("TEMPLATES_CATALOG_URL")
        .unwrap_or_else(|_| DEFAULT_CATALOG_ORIGIN.into())
        .trim_end_matches('/')
        .to_owned()
}

pub struct CatalogClient {
    client: reqwest::Client,
    base_url: String,
}
impl CatalogClient {
    pub fn new() -> Result<Self, AppError> {
        let base_url = format!("{}{CATALOG_PATH}", catalog_origin());
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| unavailable())?,
            base_url,
        })
    }
    async fn request<T: for<'de> Deserialize<'de>>(&self, url: String) -> Result<T, AppError> {
        tracing::info!(catalog_url = %url, "Fetching template catalog");
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|_| unavailable())?;
        if !response.status().is_success() {
            return Err(unavailable());
        }
        let bytes = response.bytes().await.map_err(|_| unavailable())?;
        if bytes.len() > MAX_BYTES {
            return Err(unavailable());
        }
        let body: Envelope<T> = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
        if body.error.is_some() {
            return Err(unavailable());
        }
        body.data.ok_or_else(unavailable)
    }
    pub async fn list(&self, query: &CatalogQuery) -> Result<Vec<CatalogTemplate>, AppError> {
        let mut url = reqwest::Url::parse(&self.base_url).map_err(|_| unavailable())?;
        query.append_to(&mut url)?;
        self.request(url.to_string()).await
    }
    pub async fn get(&self, id: &str) -> Result<CatalogTemplate, AppError> {
        validate_id(id)?;
        self.request(format!(
            "{}/{id}?features={CATALOG_FEATURES}",
            self.base_url
        ))
        .await
    }
}
fn unavailable() -> AppError {
    AppError::CatalogUnavailable("Template catalog is unavailable".into())
}
fn validate_id(id: &str) -> Result<(), AppError> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .enumerate()
            .all(|(i, b)| b.is_ascii_lowercase() || b.is_ascii_digit() || (i > 0 && b == b'-'))
    {
        return Err(AppError::ValidationError("Invalid template ID".into()));
    }
    Ok(())
}
pub fn trusted_icon_url(id: &str, icon: Option<&CatalogIcon>) -> Result<Option<String>, AppError> {
    validate_id(id)?;
    let Some(icon) = icon else { return Ok(None) };
    let expected = format!("/api/v1/templates/{id}/icon");
    if icon.path == expected {
        return Ok(Some(format!("{}{expected}", catalog_origin())));
    }

    let url = Url::parse(&icon.path)
        .map_err(|_| AppError::ValidationError("Template icon path is invalid".into()))?;
    if url.scheme() != "https"
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(AppError::ValidationError(
            "Template icon path is invalid".into(),
        ));
    }
    Ok(Some(url.into()))
}
/// Resolves the catalog-provided `ui` path into the absolute URL used by the
/// template redirect buttons. Relative paths hang off the catalog origin,
/// absolute ones must be https. Anything else is dropped instead of failing the
/// whole listing.
pub fn trusted_ui_url(id: &str, ui: Option<&str>) -> Result<Option<String>, AppError> {
    validate_id(id)?;
    let Some(ui) = ui.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if let Some(path) = ui.strip_prefix('/') {
        if path.starts_with('/')
            || path.contains("..")
            || ui.chars().any(|c| c.is_whitespace() || c == '\\')
        {
            return Ok(None);
        }
        return Ok(Some(format!("{}{ui}", catalog_origin())));
    }
    let Ok(url) = Url::parse(ui) else {
        return Ok(None);
    };
    if url.scheme() != "https"
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Ok(None);
    }
    Ok(Some(url.into()))
}
pub fn decode(template: CatalogTemplate) -> Result<DecodedTemplate, AppError> {
    validate_id(&template.id)?;
    let manifest_file = template
        .manifest
        .as_ref()
        .ok_or_else(|| AppError::ValidationError("Template has no manifest file".into()))?;
    let manifest = String::from_utf8(
        STANDARD
            .decode(&manifest_file.content)
            .map_err(|_| AppError::ValidationError("Template manifest file is invalid".into()))?,
    )
    .map_err(|_| AppError::ValidationError("Template manifest file is not UTF-8".into()))?;
    let file = template
        .compose
        .as_ref()
        .ok_or_else(|| AppError::ValidationError("Template has no Compose file".into()))?;
    let compose = String::from_utf8(
        STANDARD
            .decode(&file.content)
            .map_err(|_| AppError::ValidationError("Template Compose file is invalid".into()))?,
    )
    .map_err(|_| AppError::ValidationError("Template Compose file is not UTF-8".into()))?;
    let environment = match &template.env {
        Some(file) => String::from_utf8(STANDARD.decode(&file.content).map_err(|_| {
            AppError::ValidationError("Template environment file is invalid".into())
        })?)
        .map_err(|_| AppError::ValidationError("Template environment file is not UTF-8".into()))?,
        None => String::new(),
    };
    let setup = match &template.setup {
        Some(file) => String::from_utf8(
            STANDARD
                .decode(&file.content)
                .map_err(|_| AppError::ValidationError("Template setup file is invalid".into()))?,
        )
        .map_err(|_| AppError::ValidationError("Template setup file is not UTF-8".into()))?,
        None => String::new(),
    };
    let icon_url = trusted_icon_url(&template.id, template.icon.as_ref())?;
    Ok(DecodedTemplate {
        template,
        manifest,
        compose,
        environment,
        setup,
        icon_url,
    })
}
fn catalog_ports(template: &CatalogTemplate) -> Vec<domain_markers::CatalogPort> {
    template
        .ports
        .iter()
        .map(|port| domain_markers::CatalogPort {
            container: port.container,
            service: port.service.clone(),
            primary: port.primary,
        })
        .collect()
}

/// Rejects a template whose `{{url:...}}` markers name a port no service owns,
/// before any application row exists.
fn validate_domain_markers(decoded: &DecodedTemplate) -> Result<(), AppError> {
    let env = deployment::parse_environment_content(&decoded.environment)?;
    domain_markers::required_routes(&env, &catalog_ports(&decoded.template), &decoded.compose)
        .map(|_| ())
        .map_err(|error| AppError::ValidationError(format!("Template domain marker: {error}")))
}

fn generated_name(id: &str) -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::rng();
    let suffix: String = (0..8)
        .map(|_| CHARS[rng.random_range(0..CHARS.len())] as char)
        .collect();
    format!("{id}-{suffix}")
}

pub async fn create_application(
    state: &SharedState,
    organization_id: &str,
    project_id: &str,
    environment_id: &str,
    template_id: &str,
    server_id: &str,
) -> Result<ApplicationDto, AppError> {
    let mut decoded = decode(CatalogClient::new()?.get(template_id).await?)?;
    decoded.environment =
        crate::templates::secrets::substitute_placeholder_secrets(&decoded.environment);
    deployment::validate_environment_content(&decoded.environment)?;
    validate_domain_markers(&decoded)?;
    let id = uuid::Uuid::now_v7().to_string();
    let application = project_cmd::create_application_with_id(
        state,
        organization_id,
        project_id,
        environment_id,
        &id,
        Some("catalog"),
        &CreateApplicationRequest {
            name: generated_name(&decoded.template.id),
            description: Some(decoded.template.description.clone()),
            server_id: Some(server_id.into()),
            server_group_id: None,
        },
    )
    .await?;
    let primary = decoded
        .template
        .ports
        .iter()
        .find(|port| port.primary)
        .or(decoded.template.ports.first());
    let primary_port = primary.map(|port| port.container);
    // A template with no declared ports (a tunnel client like cloudflared, a
    // worker, a cron job) is legitimate: it only makes outbound connections
    // and has nothing to publish through the proxy. Only a *declared* port
    // that no Compose service actually exposes is an error.
    let public_service = match primary {
        Some(port) => Some(match &port.service {
            Some(service) => service.clone(),
            None => crate::deployment::compose_standalone::service_for_port(
                &decoded.compose,
                port.container,
            )
            .ok_or_else(|| {
                AppError::ValidationError(format!(
                    "No Compose service exposes the template's primary port {}",
                    port.container
                ))
            })?,
        }),
        None => None,
    };
    let result = async {
        // Store the template's Compose file verbatim - comments and all, so a
        // template that ships commented-out optional services (Loki alongside
        // Grafana, for instance) keeps them for the user to enable. The shared
        // `litepod_proxy` network, per-service `container_name`, and the routed
        // service's alias are layered on at deploy time by
        // `generate_standalone_override`; nothing needs to rewrite (and thereby
        // flatten) the template here.
        ComposeFileStore::new(&state.data_dir, &id)
            .write_content(&decoded.compose)
            .await
            .map_err(AppError::InternalError)?;
        source::save(
            state,
            &id,
            UpdateSourceRequest {
                deploy_type: "compose".into(),
                build_method: "compose".into(),
                git_provider_id: None,
                git_url: None,
                git_branch: None,
                auto_deploy: Some(false),
                watch_paths: None,
                dockerfile_path: None,
                compose_file_path: None,
                compose_public_service: public_service.clone(),
                container_port: None,
                railpack_project_path: Some(".".into()),
                railpack_start_command: None,
                build_cpu_cores: None,
                build_memory_mib: None,
                build_swap_mib: None,
                image_url: None,
                registry_username: None,
                registry_password: None,
                restart_policy: None,
            },
        )
        .await?;
        deployment::save_environment(state, &id, "runtime", &decoded.environment).await?;
        let conn = crate::git::repositories::db::get_conn(state).await;
        project::set_catalog_metadata(
            &conn,
            &id,
            &decoded.template.id,
            decoded.icon_url.as_deref(),
            primary_port,
            Utc::now().timestamp_millis(),
        )
        .await
        .map_err(AppError::DatabaseError)?;
        project::set_catalog_ports(
            &conn,
            &id,
            &catalog_ports(&decoded.template),
            decoded.template.requires_https,
            decoded.template.url_persisted,
        )
        .await
        .map_err(AppError::DatabaseError)
    }
    .await;
    if let Err(error) = result {
        let _ = project_cmd::delete_application(
            state,
            organization_id,
            project_id,
            environment_id,
            &id,
        )
        .await;
        let _ = ComposeFileStore::new(&state.data_dir, &id)
            .remove_all()
            .await;
        return Err(error);
    }
    Ok(application)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_templates_reject_invalid_icon_path() {
        assert!(
            trusted_icon_url(
                "mailpit",
                Some(&CatalogIcon {
                    path: "/api/v1/templates/other/icon".into()
                })
            )
            .is_err()
        );
    }

    #[test]
    fn catalog_templates_accept_https_icon() {
        assert_eq!(
            trusted_icon_url("uptime-kuma", Some(&CatalogIcon { path: "https://raw.githubusercontent.com/louislam/uptime-kuma/master/public/icon.svg".into() })).unwrap(),
            Some("https://raw.githubusercontent.com/louislam/uptime-kuma/master/public/icon.svg".into())
        );
    }

    #[test]
    fn catalog_templates_reject_non_https_icons() {
        assert!(
            trusted_icon_url(
                "mailpit",
                Some(&CatalogIcon {
                    path: "http://bad.test/icon".into()
                })
            )
            .is_err()
        );
        assert!(
            trusted_icon_url(
                "mailpit",
                Some(&CatalogIcon {
                    path: "https://user:pass@bad.test/icon".into()
                })
            )
            .is_err()
        );
    }

    #[test]
    fn catalog_templates_resolve_relative_ui_path_against_origin() {
        assert_eq!(
            trusted_ui_url("mailpit", Some("/api/v1/templates/mailpit")).unwrap(),
            Some(format!("{}/api/v1/templates/mailpit", catalog_origin()))
        );
    }

    #[test]
    fn catalog_templates_accept_https_ui_url() {
        assert_eq!(
            trusted_ui_url("mailpit", Some("https://templates.litepod.sh/t/mailpit")).unwrap(),
            Some("https://templates.litepod.sh/t/mailpit".into())
        );
    }

    #[test]
    fn catalog_templates_drop_unsafe_ui_paths() {
        for value in [
            "",
            "  ",
            "//evil.test/x",
            "/api/../../etc/passwd",
            "http://bad.test/t/mailpit",
            "javascript:alert(1)",
        ] {
            assert_eq!(trusted_ui_url("mailpit", Some(value)).unwrap(), None);
        }
        assert_eq!(trusted_ui_url("mailpit", None).unwrap(), None);
    }

    fn base64(s: &str) -> String {
        STANDARD.encode(s.as_bytes())
    }

    fn minimal_template(setup: Option<CatalogFile>) -> CatalogTemplate {
        CatalogTemplate {
            id: "mailpit".into(),
            name: "Mailpit".into(),
            description: "desc".into(),
            version: "1.0".into(),
            categories: vec![],
            tags: vec![],
            ports: vec![],
            icon: None,
            ui: None,
            website: None,
            docs: None,
            source: None,
            license: None,
            manifest: Some(CatalogFile {
                filename: "template.toml".into(),
                content: base64("[template]\nid = \"mailpit\"\nname = \"Mailpit\"\n"),
            }),
            compose: Some(CatalogFile {
                filename: "compose.yml".into(),
                content: base64("services:\n  mailpit:\n    image: docker.io/axllent/mailpit:v1\n"),
            }),
            env: None,
            setup,
            requires_https: false,
            url_persisted: false,
        }
    }

    fn ente_like(env: &str) -> DecodedTemplate {
        let mut template = minimal_template(None);
        template.ports = vec![
            CatalogPort {
                container: 3000,
                description: "web".into(),
                primary: true,
                service: None,
            },
            CatalogPort {
                container: 8080,
                description: "api".into(),
                primary: false,
                service: None,
            },
        ];
        DecodedTemplate {
            template,
            manifest: String::new(),
            compose: "services:\n  web:\n    expose: [\"3000\"]\n  api:\n    expose: [\"8080\"]\n"
                .into(),
            environment: env.into(),
            setup: String::new(),
            icon_url: None,
        }
    }

    #[test]
    fn create_validation_accepts_declared_ports() {
        assert!(validate_domain_markers(&ente_like("API={{url:8080}}\nWEB={{url}}\n")).is_ok());
    }

    #[test]
    fn create_validation_rejects_undeclared_ports() {
        assert!(matches!(
            validate_domain_markers(&ente_like("API={{url:9999}}\n")),
            Err(AppError::ValidationError(_))
        ));
    }

    #[test]
    fn catalog_template_reads_url_flags_and_port_service() {
        let template: CatalogTemplate = serde_json::from_str(
            r#"{"id":"ente","name":"Ente","description":"","version":"1","requiresHttps":true,"ports":[{"container":8080,"service":"ente-museum"}]}"#,
        )
        .unwrap();
        assert!(template.requires_https);
        assert!(!template.url_persisted);
        assert_eq!(template.ports[0].service.as_deref(), Some("ente-museum"));
    }

    #[test]
    fn decode_returns_empty_setup_when_upstream_omits_it() {
        let decoded = decode(minimal_template(None)).expect("decodes");
        assert_eq!(decoded.setup, "");
    }

    #[test]
    fn decode_returns_decoded_setup_markdown_when_present() {
        let md = "# Hello\n\nSome **setup** notes.\n";
        let decoded = decode(minimal_template(Some(CatalogFile {
            filename: "setup.md".into(),
            content: base64(md),
        })))
        .expect("decodes");
        assert_eq!(decoded.setup, md);
    }

    #[test]
    fn decode_rejects_setup_with_invalid_base64() {
        let result = decode(minimal_template(Some(CatalogFile {
            filename: "setup.md".into(),
            content: "!!!not base64!!!".into(),
        })));
        assert!(matches!(result, Err(AppError::ValidationError(_))));
    }

    #[test]
    fn decode_rejects_setup_with_non_utf8_bytes() {
        let bad_bytes = [0xff, 0xfe, 0xfd];
        let result = decode(minimal_template(Some(CatalogFile {
            filename: "setup.md".into(),
            content: STANDARD.encode(bad_bytes),
        })));
        assert!(matches!(result, Err(AppError::ValidationError(_))));
    }

    #[test]
    fn catalog_query_encodes_search_parameters() {
        let query = CatalogQuery {
            q: Some("mail test".into()),
            category: Some("email".into()),
            limit: Some(24),
        };
        let mut url = reqwest::Url::parse("https://example.test/api/v1/templates").unwrap();
        query.append_to(&mut url).unwrap();
        assert_eq!(
            url.query(),
            Some("features=domain-markers&q=mail+test&category=email&limit=24")
        );
    }

    #[test]
    fn catalog_query_rejects_limits_outside_the_upstream_range() {
        let query = CatalogQuery {
            limit: Some(0),
            ..Default::default()
        };
        let mut url = reqwest::Url::parse("https://example.test/api/v1/templates").unwrap();
        assert!(query.append_to(&mut url).is_err());
    }

    #[test]
    fn decode_preserves_compose_comments_verbatim() {
        let compose = "services:\n  grafana:\n    image: docker.io/grafana/grafana:11\n# loki:\n#   image: docker.io/grafana/loki:3\n";
        let mut template = minimal_template(None);
        template.compose = Some(CatalogFile {
            filename: "compose.yml".into(),
            content: base64(compose),
        });
        let decoded = decode(template).expect("decodes");
        assert_eq!(decoded.compose, compose);
        assert!(decoded.compose.contains("# loki:"));
    }
}
