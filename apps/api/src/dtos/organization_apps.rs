use serde::Serialize;
use utoipa::ToSchema;

/// The application fields the organization-wide lists (home, apps, domains,
/// analytics) actually render. A full `ApplicationDto` drags its server and
/// server-group summaries along, none of which those lists show.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AppSummaryApplicationDto {
    pub id: String,
    pub name: String,
    pub template_kind: Option<String>,
    pub catalog_icon_url: Option<String>,
    pub icon_url: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AppSummaryProjectDto {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AppSummaryEnvironmentDto {
    pub id: String,
    pub name: String,
    pub is_production: bool,
}

/// One domain as the org-wide lists show it: enough to link and label a host,
/// without the certificate and Caddy route probing that `DomainDto` pays for
/// per domain.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AppSummaryDomainDto {
    pub id: String,
    pub host: String,
    pub port: i64,
    pub https: bool,
    pub is_primary: bool,
    /// Fully resolved address, e.g. `https://ghost.example.com`.
    pub url: String,
    /// Name of the saved certificate the domain uses, when it uses one.
    pub certificate_name: Option<String>,
}

/// Running/total containers for an application.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AppContainersDto {
    pub running: i64,
    pub total: i64,
}

/// One row of the organization app summary: everything the org-wide lists need
/// about a single application, fetched in bulk so the lists stop paying one
/// request per application.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AppSummaryDto {
    pub application: AppSummaryApplicationDto,
    pub project: AppSummaryProjectDto,
    pub environment: AppSummaryEnvironmentDto,
    /// `application_deployments.status`, aggregated across replicas for a
    /// group-targeted application. `not_deployed` before the first deploy.
    pub status: String,
    /// Last successful deploy, epoch ms; `None` when never deployed.
    pub deployed_at: Option<i64>,
    pub domains: Vec<AppSummaryDomainDto>,
    /// `None` when the application's server could not be reached, so the UI can
    /// tell "no containers" (`0/0`) from "could not look".
    pub containers: Option<AppContainersDto>,
}
