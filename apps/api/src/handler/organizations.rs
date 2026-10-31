use axum::{
    Extension, Json,
    extract::{Multipart, Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use crate::cqrs::commands::organization as organization_cmd;
use crate::cqrs::queries::organization as organization_q;
use crate::dtos::audit_log::AuditLogFilters;
use crate::dtos::organization::{
    CreateOrganizationRequest, UpdateMemberRoleRequest, UpdateOrganizationRequest,
};
use crate::dtos::response::{AppError, ok};
use crate::git::repositories::audit_log;
use crate::icons;
use crate::security::authz::extract::{RequireOrg, perm};
use crate::security::middleware::auth::AuthUser;
use crate::state::state::SharedState;

// ── Organization CRUD ─────────────────────────────────────────────────────────────────

#[utoipa::path(
    post, path = "/api/v1/organizations", request_body = CreateOrganizationRequest,
    responses((status = 201, description = "Organization created")),
    tag = "organizations"
)]
pub async fn create_organization_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<CreateOrganizationRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Organizations are unlimited under the base + per-server billing model —
    // only remote servers are billed, so no capacity check here anymore.
    let organization =
        organization_cmd::create_organization(&state, &auth_user.user_id, &req).await?;
    Ok((axum::http::StatusCode::CREATED, Json(organization)))
}

#[utoipa::path(
    get, path = "/api/v1/organizations",
    responses((status = 200, description = "List of user's organizations")),
    tag = "organizations"
)]
pub async fn list_organizations_handler(
    State(state): State<SharedState>,
    Extension(auth_user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    let organizations = if auth_user.is_sysadmin {
        organization_q::list_all_organizations(&state).await?
    } else {
        organization_q::list_my_organizations(&state, &auth_user.user_id).await?
    };
    Ok(ok(organizations))
}

#[utoipa::path(
    get, path = "/api/v1/organizations/{organization_id}",
    responses((status = 200, description = "Organization details")),
    tag = "organizations"
)]
pub async fn get_organization_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let organization = organization_q::get_organization(&state, &organization_id).await?;
    Ok(ok(organization))
}

#[utoipa::path(
    patch, path = "/api/v1/organizations/{organization_id}", request_body = UpdateOrganizationRequest,
    responses((status = 200, description = "Organization updated")),
    tag = "organizations"
)]
pub async fn update_organization_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgUpdate>,
    Json(req): Json<UpdateOrganizationRequest>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    if req.require_two_factor == Some(true) {
        organization_cmd::ensure_can_require_two_factor(
            &state,
            &org.user.user_id,
            org.user.is_sysadmin,
        )
        .await?;
    }
    let organization =
        organization_cmd::update_organization(&state, &organization_id, &req).await?;
    audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "organization.updated",
        Some("organization"),
        Some(&organization_id),
        Some(&organization.name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    // The requirement locks members out, so its changes get a row of their own
    // rather than hiding inside a generic "organization.updated".
    if let Some(required) = req.require_two_factor {
        audit_log::record(
            &state,
            &organization_id,
            &org.user,
            "organization.two_factor_requirement_changed",
            Some("organization"),
            Some(&organization_id),
            Some(&organization.name),
            Some(&format!("{{\"required\":{required}}}")),
            None,
            None,
            None,
        )
        .await
        .ok();
    }
    Ok(ok(organization))
}

#[utoipa::path(
    delete, path = "/api/v1/organizations/{organization_id}",
    responses((status = 200, description = "Organization deleted")),
    tag = "organizations"
)]
pub async fn delete_organization_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgUpdate>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    organization_cmd::delete_organization(&state, &organization_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Members ───────────────────────────────────────────────────────────────────

#[utoipa::path(
    get, path = "/api/v1/organizations/{organization_id}/members",
    responses((status = 200, description = "Organization members")),
    tag = "organizations"
)]
pub async fn list_members_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let include_two_factor = org.permissions.allows(
        crate::security::authz::Permission::OrgMembersManage,
        crate::security::authz::Target::Org,
    );
    let members =
        organization_q::list_members(&state, &organization_id, include_two_factor).await?;
    Ok(ok(members))
}

#[utoipa::path(
    patch, path = "/api/v1/organizations/{organization_id}/members/{user_id}", request_body = UpdateMemberRoleRequest,
    responses((status = 200, description = "Member role updated")),
    tag = "organizations"
)]
pub async fn update_member_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgMembersManage>,
    // The extractor covers `organization_id`.
    Path((_organization_id, target_user_id)): Path<(String, String)>,
    Json(req): Json<UpdateMemberRoleRequest>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    organization_cmd::update_member_role(
        &state,
        &org.permissions,
        &organization_id,
        &target_user_id,
        req.role_id,
    )
    .await?;
    let target_email = crate::git::repositories::user::get_user_by_id(&state, &target_user_id)
        .await
        .map(|u| u.email);
    audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "member.role_changed",
        Some("user"),
        Some(&target_user_id),
        target_email.as_deref(),
        Some(&format!("{{\"role_id\":{}}}", req.role_id)),
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(()))
}

#[utoipa::path(
    delete, path = "/api/v1/organizations/{organization_id}/members/{user_id}",
    responses((status = 200, description = "Member removed")),
    tag = "organizations"
)]
pub async fn remove_member_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgMembersManage>,
    // The extractor covers `organization_id`.
    Path((_organization_id, target_user_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let target_email = crate::git::repositories::user::get_user_by_id(&state, &target_user_id)
        .await
        .map(|u| u.email);
    organization_cmd::remove_member(&state, &org.permissions, &organization_id, &target_user_id)
        .await?;
    audit_log::record(
        &state,
        &organization_id,
        &org.user,
        "member.removed",
        Some("user"),
        Some(&target_user_id),
        target_email.as_deref(),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(StatusCode::NO_CONTENT)
}

// ── Audit logs ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ListAuditLogsQuery {
    pub limit: Option<i64>,
    pub before: Option<String>,
    pub action: Option<String>,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub actor_user_id: Option<String>,
    pub outcome: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[utoipa::path(
    get, path = "/api/v1/organizations/{organization_id}/audit-logs",
    responses((status = 200, description = "Audit log entries, newest first")),
    tag = "organizations"
)]
pub async fn list_audit_logs_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgAuditRead>,
    Query(q): Query<ListAuditLogsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let filters = AuditLogFilters {
        action: q.action.as_deref(),
        target_type: q.target_type.as_deref(),
        target_id: q.target_id.as_deref(),
        actor_user_id: q.actor_user_id.as_deref(),
        outcome: q.outcome.as_deref(),
        from: q.from.as_deref(),
        to: q.to.as_deref(),
    };
    let logs = audit_log::list_for_organization(
        &state,
        &org.organization_id,
        limit,
        q.before.as_deref(),
        &filters,
    )
    .await
    .map_err(AppError::DatabaseError)?;
    Ok(ok(logs))
}

#[derive(Deserialize)]
pub struct ExportAuditLogsQuery {
    /// Inclusive `YYYY-MM-DD HH:MM:SS` bounds (SQLite's own `datetime()`
    /// format) — the UI computes these from its day-range presets so the
    /// comparison stays a plain string comparison here.
    pub from: Option<String>,
    pub to: Option<String>,
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[utoipa::path(
    get, path = "/api/v1/organizations/{organization_id}/audit-logs/export",
    responses((status = 200, description = "Audit log entries as CSV")),
    tag = "organizations"
)]
pub async fn export_audit_logs_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgAuditRead>,
    Query(q): Query<ExportAuditLogsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let logs = audit_log::list_for_export(
        &state,
        &org.organization_id,
        q.from.as_deref(),
        q.to.as_deref(),
    )
    .await
    .map_err(AppError::DatabaseError)?;
    let mut csv = String::from(
        "created_at,actor_email,actor_name,action,target_type,target_id,metadata,id\n",
    );
    for entry in &logs {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            csv_field(&entry.created_at),
            csv_field(entry.actor_email.as_deref().unwrap_or("")),
            csv_field(entry.actor_name.as_deref().unwrap_or("")),
            csv_field(&entry.action),
            csv_field(entry.target_type.as_deref().unwrap_or("")),
            csv_field(entry.target_id.as_deref().unwrap_or("")),
            csv_field(entry.metadata.as_deref().unwrap_or("")),
            csv_field(&entry.id),
        ));
    }
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"audit-logs.csv\"".to_string(),
            ),
        ],
        csv,
    ))
}

// ── Icon ─────────────────────────────────────────────────────────────────────

#[utoipa::path(
    post, path = "/api/v1/organizations/{organization_id}/icon",
    responses((status = 200, description = "Organization icon uploaded")),
    tag = "organizations"
)]
pub async fn upload_icon_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgUpdate>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    let organization_id = org.organization_id;
    let field = multipart
        .next_field()
        .await
        .map_err(|e| AppError::ValidationError(format!("Invalid multipart upload: {e}")))?
        .ok_or_else(|| AppError::ValidationError("Choose one icon to upload".into()))?;
    if field.name() != Some("icon") {
        return Err(AppError::ValidationError(
            "Upload field must be named icon".into(),
        ));
    }
    let bytes = field
        .bytes()
        .await
        .map_err(|e| AppError::ValidationError(format!("Could not read upload: {e}")))?;
    if bytes.len() > icons::MAX_ICON_BYTES {
        return Err(AppError::ValidationError(format!(
            "Icon exceeds the {}MiB upload limit",
            icons::MAX_ICON_BYTES / (1024 * 1024)
        )));
    }
    if multipart
        .next_field()
        .await
        .map_err(|e| AppError::ValidationError(format!("Invalid multipart upload: {e}")))?
        .is_some()
    {
        return Err(AppError::ValidationError("Upload exactly one icon".into()));
    }

    let filename = icons::store(&state, "organizations", &organization_id, &bytes).await?;
    let icon_url = format!("/api/v1/organizations/{organization_id}/icon/{filename}");
    let organization = organization_cmd::update_organization(
        &state,
        &organization_id,
        &UpdateOrganizationRequest {
            name: None,
            description: None,
            icon: Some(icon_url),
            icon_bg: None,
            plan: None,
            audit_logs_enabled: None,
            assistant_enabled: None,
            mcp_enabled: None,
            require_two_factor: None,
            audit_logs_retention_days: None,
        },
    )
    .await?;
    Ok(ok(organization))
}

// No #[utoipa::path] here — utoipa's macro can't resolve a destructured tuple
// `Path` pattern (same reason `update_domain_handler` skips it).
pub async fn serve_icon_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgRead>,
    // The extractor covers `organization_id`.
    Path((_, filename)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let bytes = icons::read(&state, "organizations", &org.organization_id, &filename).await?;
    let content_type = icons::content_type_for(&filename);
    let mut response = Response::new(axum::body::Body::from(bytes));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static(content_type),
    );
    headers.insert(
        header::HeaderName::from_static("x-content-type-options"),
        axum::http::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    if content_type == "image/svg+xml" {
        headers.insert(
            header::HeaderName::from_static("content-security-policy"),
            axum::http::HeaderValue::from_static("script-src 'none'"),
        );
    }
    Ok(response)
}
