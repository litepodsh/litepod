use crate::{
    dtos::{
        catalog_template::{CatalogFileDto, CatalogPortDto, CatalogTemplateDto},
        response::{AppError, created, ok},
    },
    security::authz::extract::{RequireOrg, perm},
    security::middleware::auth::AuthUser,
    state::state::SharedState,
    templates::catalog_templates::{self, CatalogClient, CatalogQuery, CatalogTemplate},
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use utoipa::ToSchema;

fn dto(template: CatalogTemplate) -> Result<CatalogTemplateDto, AppError> {
    let icon_url = catalog_templates::trusted_icon_url(&template.id, template.icon.as_ref())?;
    let ui_url = catalog_templates::trusted_ui_url(&template.id, template.ui.as_deref())?;
    Ok(CatalogTemplateDto {
        requires_https: template.requires_https,
        url_persisted: template.url_persisted,
        id: template.id,
        name: template.name,
        description: template.description,
        version: template.version,
        categories: template.categories,
        tags: template.tags,
        ports: template
            .ports
            .into_iter()
            .map(|p| CatalogPortDto {
                container: p.container,
                description: p.description,
                primary: p.primary,
                service: p.service,
            })
            .collect(),
        icon_url,
        ui_url,
        website: template.website,
        docs: template.docs,
        source: template.source,
        license: template.license,
        manifest: None,
        compose: None,
        env: None,
        setup: None,
    })
}
fn detail_dto(template: CatalogTemplate) -> Result<CatalogTemplateDto, AppError> {
    let manifest_filename = template
        .manifest
        .as_ref()
        .ok_or_else(|| AppError::ValidationError("Template has no manifest file".into()))?
        .filename
        .clone();
    let compose_filename = template
        .compose
        .as_ref()
        .ok_or_else(|| AppError::ValidationError("Template has no Compose file".into()))?
        .filename
        .clone();
    let environment_filename = template.env.as_ref().map(|file| file.filename.clone());
    let setup_filename = template.setup.as_ref().map(|file| file.filename.clone());
    let decoded = catalog_templates::decode(template)?;
    let mut result = dto(decoded.template)?;
    result.manifest = Some(CatalogFileDto {
        filename: manifest_filename,
        content: decoded.manifest,
    });
    result.compose = Some(CatalogFileDto {
        filename: compose_filename,
        content: decoded.compose,
    });
    result.env = environment_filename.map(|filename| CatalogFileDto {
        filename,
        content: decoded.environment,
    });
    if !decoded.setup.is_empty() {
        let filename = setup_filename.unwrap_or_else(|| "setup.md".into());
        result.setup = Some(CatalogFileDto {
            filename,
            content: decoded.setup,
        });
    }
    Ok(result)
}
pub async fn list_handler(
    Query(query): Query<CatalogQuery>,
    Extension(_auth): Extension<AuthUser>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let client = CatalogClient::new()?;
    Ok(ok(client
        .list(&query)
        .await?
        .into_iter()
        .map(dto)
        .collect::<Result<Vec<_>, _>>()?))
}
pub async fn get_handler(
    Path(template_id): Path<String>,
    Extension(_auth): Extension<AuthUser>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    Ok(ok(detail_dto(
        CatalogClient::new()?.get(&template_id).await?,
    )?))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateCatalogTemplateRequest {
    pub server_id: String,
}

pub async fn create_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::AppCreate>,
    // The extractor covers `organization_id`; the rest of the path is still needed.
    // `create_application` scopes project/environment to the organization itself.
    Path((_organization_id, project_id, environment_id, template_id)): Path<(
        String,
        String,
        String,
        String,
    )>,
    Json(request): Json<CreateCatalogTemplateRequest>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let application = catalog_templates::create_application(
        &state,
        &organization_id,
        &project_id,
        &environment_id,
        &template_id,
        &request.server_id,
    )
    .await?;
    let _ = crate::git::repositories::audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "application.created",
        Some("application"),
        Some(&application.id),
        Some(&application.name),
        Some(&template_id),
        None,
        None,
        None,
    )
    .await;
    crate::handler::dashboard::invalidate_organization_dashboard(&state, &organization_id).await;
    Ok(created(application))
}
