use axum::{
    Json,
    extract::{Path, State},
    response::IntoResponse,
};
use serde_json::from_str;
use uuid::Uuid;

use crate::security::authz::extract::{RequireOrg, perm};
use crate::{
    dtos::{
        response::{AppError, created, ok},
        server::ServerSummaryDto,
        server_group::{
            CreateServerGroupRequest, ReplaceServerGroupMembersRequest, ServerGroupDto,
            UpdateServerGroupRequest,
        },
    },
    servers::{
        groups::{self, ServerGroupRecord},
        setup::ServerFacts,
    },
    state::state::SharedState,
};

const MAX_NAME_LEN: usize = 100;
const MAX_DESCRIPTION_LEN: usize = 2000;

fn validate_name(name: &str) -> Result<&str, AppError> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_NAME_LEN {
        return Err(AppError::ValidationError(format!(
            "Server group name must be between 1 and {MAX_NAME_LEN} characters"
        )));
    }
    Ok(trimmed)
}

fn validate_description(description: &str) -> Result<&str, AppError> {
    let trimmed = description.trim();
    if trimmed.len() > MAX_DESCRIPTION_LEN {
        return Err(AppError::ValidationError(format!(
            "Server group description must be at most {MAX_DESCRIPTION_LEN} characters"
        )));
    }
    Ok(trimmed)
}

fn member_summary(server: &crate::servers::repository::ServerRecord) -> ServerSummaryDto {
    let facts = from_str::<ServerFacts>(&server.facts_json).unwrap_or_default();
    crate::dtos::server::server_summary(server, &facts)
}

async fn full_dto(
    state: &SharedState,
    group: ServerGroupRecord,
) -> Result<ServerGroupDto, AppError> {
    let members = groups::list_members(state, &group.id).await?;
    let member_servers = members
        .iter()
        .map(|server| member_summary(server))
        .collect::<Vec<_>>();
    Ok(ServerGroupDto {
        member_count: member_servers.len() as u64,
        member_servers,
        id: group.id,
        name: group.name,
        description: group.description,
        created_at: group.created_at,
        updated_at: group.updated_at,
    })
}

pub async fn list_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
) -> Result<impl IntoResponse, AppError> {
    let groups = groups::list(&state, &org.organization_id).await?;
    let mut dtos = Vec::with_capacity(groups.len());
    for group in groups {
        dtos.push(full_dto(&state, group).await?);
    }
    Ok(ok(dtos))
}

pub async fn create_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Json(request): Json<CreateServerGroupRequest>,
) -> Result<impl IntoResponse, AppError> {
    let name = validate_name(&request.name)?;
    let description = validate_description(request.description.as_deref().unwrap_or(""))?;
    let group = groups::insert(
        &state,
        &Uuid::now_v7().to_string(),
        &org.organization_id,
        name,
        description,
    )
    .await?;
    crate::git::repositories::audit_log::record(
        &state,
        &org.organization_id,
        &org.user,
        "server_group.created",
        Some("server_group"),
        Some(&group.id),
        Some(&group.name),
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(created(full_dto(&state, group).await?))
}

pub async fn get_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
    Path((_, group_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let group = groups::get_for_organization(&state, &group_id, &org.organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    Ok(ok(full_dto(&state, group).await?))
}

pub async fn update_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, group_id)): Path<(String, String)>,
    Json(request): Json<UpdateServerGroupRequest>,
) -> Result<impl IntoResponse, AppError> {
    let existing = groups::get_for_organization(&state, &group_id, &org.organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    let name = match request.name {
        Some(name) => validate_name(&name)?.to_string(),
        None => existing.name,
    };
    let description = match request.description {
        Some(description) => validate_description(&description)?.to_string(),
        None => existing.description,
    };
    let group =
        groups::update(&state, &group_id, &org.organization_id, &name, &description).await?;
    Ok(ok(full_dto(&state, group).await?))
}

pub async fn delete_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, group_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    groups::get_for_organization(&state, &group_id, &org.organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    groups::delete(&state, &group_id, &org.organization_id).await?;
    crate::git::repositories::audit_log::record(
        &state,
        &org.organization_id,
        &org.user,
        "server_group.deleted",
        Some("server_group"),
        Some(&group_id),
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .ok();
    Ok(ok(()))
}

pub async fn list_members_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerRead>,
    Path((_, group_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    groups::get_for_organization(&state, &group_id, &org.organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    let members = groups::list_members(&state, &group_id).await?;
    Ok(ok(members
        .iter()
        .map(|server| member_summary(server))
        .collect::<Vec<_>>()))
}

pub async fn replace_members_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::ServerManage>,
    Path((_, group_id)): Path<(String, String)>,
    Json(request): Json<ReplaceServerGroupMembersRequest>,
) -> Result<impl IntoResponse, AppError> {
    groups::get_for_organization(&state, &group_id, &org.organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    groups::replace_members(&state, &group_id, &org.organization_id, &request.server_ids).await?;
    let group = groups::get_for_organization(&state, &group_id, &org.organization_id)
        .await?
        .ok_or(AppError::ResourceNotFound)?;
    Ok(ok(full_dto(&state, group).await?))
}
