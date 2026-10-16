//! SCIM 2.0 (RFC 7643/7644), only the subset Microsoft Entra and Okta use
//! (spec §4). Base `/scim/v2/{organization_id}`, bearer token per org.
//!
//! A SCIM tenant only sees and creates users under its organization's
//! verified domains (plus its existing members): a leaked token must not
//! be able to mint or reach accounts for anyone else's addresses.

use axum::{
    Json,
    extract::{FromRequestParts, Path, Query, State},
    http::{StatusCode, header, request::Parts},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{Row, SqliteConnection};
use uuid::Uuid;

use super::provision::create_user;
use super::{db, domain_is_verified, email_domain};
use crate::dtos::response::AppError;
use crate::git::repositories::audit_log;
use crate::security::middleware::auth::AuthUser;
use crate::security::middleware::org_scope::path_param;
use crate::state::state::SharedState;
use crate::utils::security;

const USER_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:User";
const GROUP_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:Group";
const LIST_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:ListResponse";
const ERROR_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:Error";

// ── Responses ───────────────────────────────────────────────────────────────

pub struct ScimError(StatusCode, String);

impl IntoResponse for ScimError {
    fn into_response(self) -> Response {
        scim(
            self.0,
            json!({ "schemas": [ERROR_SCHEMA], "status": self.0.as_u16().to_string(), "detail": self.1 }),
        )
    }
}

impl From<AppError> for ScimError {
    fn from(e: AppError) -> Self {
        let status = match &e {
            AppError::AlreadyExists => StatusCode::CONFLICT,
            AppError::ResourceNotFound => StatusCode::NOT_FOUND,
            AppError::ValidationError(_) => StatusCode::BAD_REQUEST,
            AppError::InvalidCredentials => StatusCode::UNAUTHORIZED,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ScimError(status, e.to_string())
    }
}

fn scim(status: StatusCode, body: Value) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/scim+json")],
        Json(body),
    )
        .into_response()
}

type ScimResult = Result<Response, ScimError>;

fn bad(detail: impl Into<String>) -> ScimError {
    ScimError(StatusCode::BAD_REQUEST, detail.into())
}

fn not_found() -> ScimError {
    ScimError(StatusCode::NOT_FOUND, "Resource not found".into())
}

// ── Auth ────────────────────────────────────────────────────────────────────

/// Proof the request carries this organization's SCIM token.
pub struct ScimTenant {
    pub organization_id: String,
}

impl FromRequestParts<SharedState> for ScimTenant {
    type Rejection = ScimError;

    async fn from_request_parts(parts: &mut Parts, state: &SharedState) -> Result<Self, ScimError> {
        let unauthorized = || ScimError(StatusCode::UNAUTHORIZED, "Invalid SCIM token".into());
        let organization_id = path_param(parts, "organization_id")
            .await
            .map_err(|_| unauthorized())?;
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(unauthorized)?;
        let stored: Option<String> =
            sqlx::query_scalar("SELECT scim_token_digest FROM organizations WHERE id = ?1")
                .bind(&organization_id)
                .fetch_optional(&state.db)
                .await
                .map_err(|e| ScimError::from(db(e)))?
                .flatten();
        match stored {
            // Digests are fixed-length hex of a 256-bit secret; comparing them leaks nothing useful.
            Some(digest) if digest == security::digest_token(token.trim()) => {
                Ok(ScimTenant { organization_id })
            }
            _ => Err(unauthorized()),
        }
    }
}

async fn audit(
    state: &SharedState,
    organization_id: &str,
    action: &str,
    id: &str,
    name: &str,
    meta: Value,
) {
    let actor = AuthUser {
        user_id: String::new(),
        email: String::new(),
        name: "SCIM".into(),
        is_sysadmin: false,
        token_id: None,
        token_name: Some("scim".into()),
    };
    audit_log::record(
        state,
        organization_id,
        &actor,
        action,
        Some("user"),
        Some(id),
        Some(name),
        Some(&meta.to_string()),
        None,
        None,
        None,
    )
    .await
    .ok();
}

// ── Discovery ───────────────────────────────────────────────────────────────

pub async fn service_provider_config(_tenant: ScimTenant) -> Response {
    scim(
        StatusCode::OK,
        json!({
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig"],
            "patch": { "supported": true },
            "bulk": { "supported": false, "maxOperations": 0, "maxPayloadSize": 0 },
            "filter": { "supported": true, "maxResults": 200 },
            "changePassword": { "supported": false },
            "sort": { "supported": false },
            "etag": { "supported": false },
            "authenticationSchemes": [{
                "type": "oauthbearertoken",
                "name": "OAuth Bearer Token",
                "description": "The organization's SCIM token from Litepod",
            }],
        }),
    )
}

pub async fn resource_types(_tenant: ScimTenant) -> Response {
    let types = json!([
        { "schemas": ["urn:ietf:params:scim:schemas:core:2.0:ResourceType"],
          "id": "User", "name": "User", "endpoint": "/Users", "schema": USER_SCHEMA },
        { "schemas": ["urn:ietf:params:scim:schemas:core:2.0:ResourceType"],
          "id": "Group", "name": "Group", "endpoint": "/Groups", "schema": GROUP_SCHEMA },
    ]);
    list_response(types.as_array().unwrap().clone(), 1)
}

pub async fn schemas(_tenant: ScimTenant) -> Response {
    let schemas = json!([
        { "id": USER_SCHEMA, "name": "User", "attributes": [
            { "name": "userName", "type": "string", "required": true, "uniqueness": "server" },
            { "name": "displayName", "type": "string" },
            { "name": "active", "type": "boolean" },
            { "name": "emails", "type": "complex", "multiValued": true },
        ]},
        { "id": GROUP_SCHEMA, "name": "Group", "attributes": [
            { "name": "displayName", "type": "string", "required": true },
            { "name": "members", "type": "complex", "multiValued": true },
        ]},
    ]);
    list_response(schemas.as_array().unwrap().clone(), 1)
}

fn list_response(resources: Vec<Value>, start_index: i64) -> Response {
    scim(
        StatusCode::OK,
        json!({
            "schemas": [LIST_SCHEMA],
            "totalResults": resources.len(),
            "startIndex": start_index,
            "itemsPerPage": resources.len(),
            "Resources": resources,
        }),
    )
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub filter: Option<String>,
    #[serde(rename = "startIndex")]
    pub start_index: Option<i64>,
    pub count: Option<i64>,
}

/// `attr eq "value"`, the only filter Entra and Okta send.
fn eq_filter<'a>(filter: &'a str, attribute: &str) -> Option<&'a str> {
    let (attr, rest) = filter.trim().split_once(' ')?;
    let (op, value) = rest.trim().split_once(' ')?;
    (attr.eq_ignore_ascii_case(attribute) && op.eq_ignore_ascii_case("eq"))
        .then(|| value.trim().trim_matches('"'))
}

fn page(mut items: Vec<Value>, query: &ListQuery) -> Response {
    let start = query.start_index.unwrap_or(1).max(1);
    let total = items.len();
    let count = query.count.unwrap_or(200).clamp(0, 200) as usize;
    let skip = (start as usize - 1).min(total);
    items = items.into_iter().skip(skip).take(count).collect();
    let mut response = json!({
        "schemas": [LIST_SCHEMA],
        "totalResults": total,
        "startIndex": start,
        "itemsPerPage": items.len(),
        "Resources": items,
    });
    if response["Resources"]
        .as_array()
        .is_some_and(|r| r.is_empty())
    {
        response["Resources"] = json!([]);
    }
    scim(StatusCode::OK, response)
}

// ── Users ───────────────────────────────────────────────────────────────────

struct ScimUser {
    id: String,
    email: String,
    name: String,
    active: bool,
}

fn user_json(u: &ScimUser) -> Value {
    json!({
        "schemas": [USER_SCHEMA],
        "id": u.id,
        "userName": u.email,
        "displayName": u.name,
        "name": { "formatted": u.name },
        "emails": [{ "value": u.email, "type": "work", "primary": true }],
        "active": u.active,
        "meta": { "resourceType": "User" },
    })
}

/// Users this tenant may see: members of the org, and anyone under one of its
/// verified domains (so a deactivated user can be found and reactivated).
async fn users(
    conn: &mut SqliteConnection,
    organization_id: &str,
    only_id: Option<&str>,
    only_email: Option<&str>,
) -> Result<Vec<ScimUser>, AppError> {
    let rows = sqlx::query(
        "SELECT u.id, u.email, u.name,
                EXISTS (SELECT 1 FROM organization_members m
                         WHERE m.organization_id = ?1 AND m.user_id = u.id) AND u.status_id = 1
           FROM users u
          WHERE (?2 IS NULL OR u.id = ?2)
            AND (?3 IS NULL OR u.email = lower(?3))
            AND (EXISTS (SELECT 1 FROM organization_members m WHERE m.organization_id = ?1 AND m.user_id = u.id)
              OR EXISTS (SELECT 1 FROM sso_domains d
                          WHERE d.organization_id = ?1 AND d.verified_at IS NOT NULL
                            AND u.email LIKE '%@' || d.domain))
          ORDER BY u.email",
    )
    .bind(organization_id)
    .bind(only_id)
    .bind(only_email)
    .fetch_all(&mut *conn)
    .await
    .map_err(db)?;
    Ok(rows
        .iter()
        .map(|r| ScimUser {
            id: r.get(0),
            email: r.get(1),
            name: r.get(2),
            active: r.get::<i64, _>(3) == 1,
        })
        .collect())
}

async fn one_user(
    conn: &mut SqliteConnection,
    organization_id: &str,
    id: &str,
) -> Result<ScimUser, ScimError> {
    users(conn, organization_id, Some(id), None)
        .await?
        .pop()
        .ok_or_else(not_found)
}

pub async fn list_users(
    State(state): State<SharedState>,
    tenant: ScimTenant,
    Query(query): Query<ListQuery>,
) -> ScimResult {
    let email = match query.filter.as_deref() {
        None => None,
        Some(f) => Some(
            eq_filter(f, "userName")
                .ok_or_else(|| bad("Only userName eq filters are supported"))?,
        ),
    };
    let mut conn = state.db.acquire().await.map_err(db)?;
    let list = users(&mut conn, &tenant.organization_id, None, email).await?;
    Ok(page(list.iter().map(user_json).collect(), &query))
}

pub async fn get_user(
    State(state): State<SharedState>,
    tenant: ScimTenant,
    Path((_organization_id, id)): Path<(String, String)>,
) -> ScimResult {
    let mut conn = state.db.acquire().await.map_err(db)?;
    Ok(scim(
        StatusCode::OK,
        user_json(&one_user(&mut conn, &tenant.organization_id, &id).await?),
    ))
}

fn email_of(body: &Value) -> Option<String> {
    body.get("userName")
        .and_then(Value::as_str)
        .or_else(|| {
            body.get("emails")?
                .as_array()?
                .iter()
                .find(|e| e.get("primary").and_then(Value::as_bool).unwrap_or(true))?
                .get("value")?
                .as_str()
        })
        .map(|e| e.trim().to_ascii_lowercase())
}

fn name_of(body: &Value) -> Option<String> {
    body.get("displayName")
        .and_then(Value::as_str)
        .or_else(|| body.get("name")?.get("formatted")?.as_str())
        .map(str::to_string)
        .or_else(|| {
            let name = body.get("name")?;
            let given = name.get("givenName").and_then(Value::as_str).unwrap_or("");
            let family = name.get("familyName").and_then(Value::as_str).unwrap_or("");
            let full = format!("{given} {family}").trim().to_string();
            (!full.is_empty()).then_some(full)
        })
}

/// SCIM booleans arrive as `false` or, from Entra, `"False"`.
fn as_bool(value: &Value) -> Option<bool> {
    value.as_bool().or_else(|| {
        value
            .as_str()
            .and_then(|s| s.to_ascii_lowercase().parse().ok())
    })
}

/// The base role a SCIM-provisioned member starts with: the first
/// connection's default, else Member.
async fn default_base_role(
    conn: &mut SqliteConnection,
    organization_id: &str,
) -> Result<i64, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT default_base_role_id FROM sso_connections WHERE organization_id = ?1 ORDER BY created_at LIMIT 1",
    )
    .bind(organization_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(db)?
    .unwrap_or(4))
}

/// Spec §4: in this org, membership on; globally, a deactivated account comes back.
async fn activate(
    conn: &mut SqliteConnection,
    organization_id: &str,
    user_id: &str,
) -> Result<(), AppError> {
    let role = default_base_role(conn, organization_id).await?;
    sqlx::query(
        "INSERT OR IGNORE INTO organization_members (organization_id, user_id, role_id) VALUES (?1, ?2, ?3)",
    )
    .bind(organization_id)
    .bind(user_id)
    .bind(role)
    .execute(&mut *conn)
    .await
    .map_err(db)?;
    sqlx::query(
        "UPDATE users SET status_id = 1, deactivated_at = NULL WHERE id = ?1 AND status_id = 2",
    )
    .bind(user_id)
    .execute(&mut *conn)
    .await
    .map_err(db)?;
    Ok(())
}

/// Spec §4: out of this org, every session and personal access token gone;
/// if no organization is left, the account is deactivated.
pub async fn deactivate(
    conn: &mut SqliteConnection,
    organization_id: &str,
    user_id: &str,
) -> Result<(), AppError> {
    // The rbac_on_member_leave trigger drops their assignments and group seats.
    sqlx::query("DELETE FROM organization_members WHERE organization_id = ?1 AND user_id = ?2")
        .bind(organization_id)
        .bind(user_id)
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    sqlx::query("DELETE FROM sessions WHERE user_id = ?1")
        .bind(user_id)
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    sqlx::query(
        "UPDATE personal_access_tokens SET revoked_at = datetime('now') WHERE user_id = ?1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(&mut *conn)
    .await
    .map_err(db)?;
    sqlx::query(
        "UPDATE users SET status_id = 2, deactivated_at = datetime('now')
          WHERE id = ?1 AND status_id != 4
            AND NOT EXISTS (SELECT 1 FROM organization_members WHERE user_id = ?1)",
    )
    .bind(user_id)
    .execute(&mut *conn)
    .await
    .map_err(db)?;
    Ok(())
}

async fn set_active(
    conn: &mut SqliteConnection,
    organization_id: &str,
    user_id: &str,
    active: bool,
) -> Result<(), AppError> {
    if active {
        activate(conn, organization_id, user_id).await
    } else {
        deactivate(conn, organization_id, user_id).await
    }
}

async fn rename(conn: &mut SqliteConnection, user_id: &str, name: &str) -> Result<(), AppError> {
    // users.name is unique; a clash keeps the old name rather than failing provisioning.
    sqlx::query(
        "UPDATE users SET name = ?1 WHERE id = ?2 AND NOT EXISTS (SELECT 1 FROM users WHERE name = ?1 AND id <> ?2)",
    )
    .bind(name.trim())
    .bind(user_id)
    .execute(&mut *conn)
    .await
    .map_err(db)?;
    Ok(())
}

pub async fn create_user_handler(
    State(state): State<SharedState>,
    tenant: ScimTenant,
    Json(body): Json<Value>,
) -> ScimResult {
    let email = email_of(&body).ok_or_else(|| bad("userName is required"))?;
    let org = &tenant.organization_id;
    let mut tx = state.db.begin().await.map_err(db)?;
    if email_domain(&email).is_none() || !domain_is_verified(&mut tx, org, &email).await? {
        return Err(bad(
            "userName must be an email under a domain this organization verified",
        ));
    }
    let existing = users(&mut tx, org, None, Some(&email)).await?.pop();
    let user_id = match existing {
        Some(user) if user.active => {
            return Err(ScimError(
                StatusCode::CONFLICT,
                "User already exists".into(),
            ));
        }
        Some(user) => user.id,
        None => create_user(&mut tx, &email, name_of(&body).as_deref().unwrap_or("")).await?,
    };
    let active = body.get("active").and_then(as_bool).unwrap_or(true);
    set_active(&mut tx, org, &user_id, active).await?;
    let user = one_user(&mut tx, org, &user_id).await?;
    tx.commit().await.map_err(db)?;
    audit(
        &state,
        org,
        "scim.user_provisioned",
        &user.id,
        &user.email,
        json!({ "active": active }),
    )
    .await;
    Ok(scim(StatusCode::CREATED, user_json(&user)))
}

pub async fn replace_user(
    State(state): State<SharedState>,
    tenant: ScimTenant,
    Path((_organization_id, id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> ScimResult {
    let org = &tenant.organization_id;
    let mut tx = state.db.begin().await.map_err(db)?;
    one_user(&mut tx, org, &id).await?;
    if let Some(name) = name_of(&body) {
        rename(&mut tx, &id, &name).await?;
    }
    let active = body.get("active").and_then(as_bool).unwrap_or(true);
    set_active(&mut tx, org, &id, active).await?;
    let user = one_user(&mut tx, org, &id).await?;
    tx.commit().await.map_err(db)?;
    audit(
        &state,
        org,
        "scim.user_replaced",
        &user.id,
        &user.email,
        json!({ "active": active }),
    )
    .await;
    Ok(scim(StatusCode::OK, user_json(&user)))
}

fn operations(body: &Value) -> Result<&Vec<Value>, ScimError> {
    body.get("Operations")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("Operations is required"))
}

pub async fn patch_user(
    State(state): State<SharedState>,
    tenant: ScimTenant,
    Path((_organization_id, id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> ScimResult {
    let org = &tenant.organization_id;
    let mut tx = state.db.begin().await.map_err(db)?;
    one_user(&mut tx, org, &id).await?;
    for op in operations(&body)? {
        let path = op.get("path").and_then(Value::as_str).unwrap_or("");
        let value = op.get("value").cloned().unwrap_or(Value::Null);
        // Either `{path: "active", value: false}` or `{value: {active: false}}`.
        let field = |name: &str| -> Option<Value> {
            if path.eq_ignore_ascii_case(name) {
                Some(value.clone())
            } else if path.is_empty() {
                value.get(name).cloned()
            } else {
                None
            }
        };
        if let Some(active) = field("active").as_ref().and_then(as_bool) {
            set_active(&mut tx, org, &id, active).await?;
        }
        if let Some(name) = field("displayName").or_else(|| field("name.formatted")) {
            if let Some(name) = name.as_str() {
                rename(&mut tx, &id, name).await?;
            }
        }
    }
    let user = one_user(&mut tx, org, &id).await?;
    tx.commit().await.map_err(db)?;
    audit(
        &state,
        org,
        "scim.user_patched",
        &user.id,
        &user.email,
        json!({ "active": user.active }),
    )
    .await;
    Ok(scim(StatusCode::OK, user_json(&user)))
}

pub async fn delete_user(
    State(state): State<SharedState>,
    tenant: ScimTenant,
    Path((_organization_id, id)): Path<(String, String)>,
) -> ScimResult {
    let org = &tenant.organization_id;
    let mut tx = state.db.begin().await.map_err(db)?;
    let user = one_user(&mut tx, org, &id).await?;
    deactivate(&mut tx, org, &id).await?;
    tx.commit().await.map_err(db)?;
    audit(
        &state,
        org,
        "scim.user_deprovisioned",
        &user.id,
        &user.email,
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ── Groups ──────────────────────────────────────────────────────────────────

async fn group_json(
    conn: &mut SqliteConnection,
    organization_id: &str,
    id: &str,
) -> Result<Value, ScimError> {
    let name: String = sqlx::query_scalar(
        "SELECT name FROM rbac_groups WHERE id = ?1 AND organization_id = ?2 AND source = 'scim'",
    )
    .bind(id)
    .bind(organization_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(db)?
    .ok_or_else(not_found)?;
    let members = sqlx::query(
        "SELECT u.id, u.email FROM rbac_group_members gm JOIN users u ON u.id = gm.user_id WHERE gm.group_id = ?1",
    )
    .bind(id)
    .fetch_all(&mut *conn)
    .await
    .map_err(db)?;
    Ok(json!({
        "schemas": [GROUP_SCHEMA],
        "id": id,
        "displayName": name,
        "members": members.iter().map(|m| json!({ "value": m.get::<String, _>(0), "display": m.get::<String, _>(1) })).collect::<Vec<_>>(),
        "meta": { "resourceType": "Group" },
    }))
}

pub async fn list_groups(
    State(state): State<SharedState>,
    tenant: ScimTenant,
    Query(query): Query<ListQuery>,
) -> ScimResult {
    let name = match query.filter.as_deref() {
        None => None,
        Some(f) => Some(
            eq_filter(f, "displayName")
                .ok_or_else(|| bad("Only displayName eq filters are supported"))?,
        ),
    };
    let mut conn = state.db.acquire().await.map_err(db)?;
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM rbac_groups WHERE organization_id = ?1 AND source = 'scim' AND (?2 IS NULL OR name = ?2) ORDER BY name",
    )
    .bind(&tenant.organization_id)
    .bind(name)
    .fetch_all(&mut *conn)
    .await
    .map_err(db)?;
    let mut groups = Vec::new();
    for id in ids {
        groups.push(group_json(&mut conn, &tenant.organization_id, &id).await?);
    }
    Ok(page(groups, &query))
}

pub async fn get_group(
    State(state): State<SharedState>,
    tenant: ScimTenant,
    Path((_organization_id, id)): Path<(String, String)>,
) -> ScimResult {
    let mut conn = state.db.acquire().await.map_err(db)?;
    Ok(scim(
        StatusCode::OK,
        group_json(&mut conn, &tenant.organization_id, &id).await?,
    ))
}

/// Adds org members to the group; ids that are not members are skipped.
async fn add_members(
    conn: &mut SqliteConnection,
    organization_id: &str,
    group_id: &str,
    members: &Value,
) -> Result<(), AppError> {
    for member in members.as_array().into_iter().flatten() {
        if let Some(user_id) = member.get("value").and_then(Value::as_str) {
            sqlx::query(
                "INSERT OR IGNORE INTO rbac_group_members (group_id, user_id)
                 SELECT ?1, user_id FROM organization_members WHERE organization_id = ?2 AND user_id = ?3",
            )
            .bind(group_id)
            .bind(organization_id)
            .bind(user_id)
            .execute(&mut *conn)
            .await
            .map_err(db)?;
        }
    }
    Ok(())
}

pub async fn create_group(
    State(state): State<SharedState>,
    tenant: ScimTenant,
    Json(body): Json<Value>,
) -> ScimResult {
    let name = body
        .get("displayName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .ok_or_else(|| bad("displayName is required"))?;
    let org = &tenant.organization_id;
    let id = Uuid::now_v7().to_string();
    let mut tx = state.db.begin().await.map_err(db)?;
    sqlx::query(
        "INSERT INTO rbac_groups (id, organization_id, name, source, external_id) VALUES (?1, ?2, ?3, 'scim', ?4)",
    )
    .bind(&id)
    .bind(org)
    .bind(name)
    .bind(body.get("externalId").and_then(Value::as_str).unwrap_or(&id))
    .execute(&mut *tx)
    .await
    .map_err(db)?;
    if let Some(members) = body.get("members") {
        add_members(&mut tx, org, &id, members).await?;
    }
    let group = group_json(&mut tx, org, &id).await?;
    tx.commit().await.map_err(db)?;
    audit(&state, org, "scim.group_created", &id, name, json!({})).await;
    Ok(scim(StatusCode::CREATED, group))
}

/// `members[value eq "id"]` → `id`.
fn member_in_path(path: &str) -> Option<&str> {
    let inner = path.strip_prefix("members[")?.strip_suffix(']')?;
    eq_filter(inner, "value")
}

pub async fn patch_group(
    State(state): State<SharedState>,
    tenant: ScimTenant,
    Path((_organization_id, id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> ScimResult {
    let org = &tenant.organization_id;
    let mut tx = state.db.begin().await.map_err(db)?;
    group_json(&mut tx, org, &id).await?;
    for op in operations(&body)? {
        let kind = op
            .get("op")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let path = op.get("path").and_then(Value::as_str).unwrap_or("");
        let value = op.get("value").cloned().unwrap_or(Value::Null);
        match (kind.as_str(), path) {
            ("add", "members") => add_members(&mut tx, org, &id, &value).await?,
            ("remove", p) if p == "members" || member_in_path(p).is_some() => {
                let ids: Vec<String> = match member_in_path(p) {
                    Some(one) => vec![one.to_string()],
                    None => value
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|m| m.get("value").and_then(Value::as_str).map(str::to_string))
                        .collect(),
                };
                for user_id in ids {
                    sqlx::query(
                        "DELETE FROM rbac_group_members WHERE group_id = ?1 AND user_id = ?2",
                    )
                    .bind(&id)
                    .bind(user_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(db)?;
                }
            }
            ("replace", "members") => {
                sqlx::query("DELETE FROM rbac_group_members WHERE group_id = ?1")
                    .bind(&id)
                    .execute(&mut *tx)
                    .await
                    .map_err(db)?;
                add_members(&mut tx, org, &id, &value).await?;
            }
            ("replace", p) => {
                let name = if p.eq_ignore_ascii_case("displayName") {
                    value.as_str().map(str::to_string)
                } else {
                    value
                        .get("displayName")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                };
                if let Some(name) = name.filter(|n| !n.trim().is_empty()) {
                    sqlx::query("UPDATE rbac_groups SET name = ?1 WHERE id = ?2")
                        .bind(name.trim())
                        .bind(&id)
                        .execute(&mut *tx)
                        .await
                        .map_err(db)?;
                }
            }
            _ => {}
        }
    }
    let group = group_json(&mut tx, org, &id).await?;
    tx.commit().await.map_err(db)?;
    audit(
        &state,
        org,
        "scim.group_patched",
        &id,
        group["displayName"].as_str().unwrap_or(""),
        json!({}),
    )
    .await;
    Ok(scim(StatusCode::OK, group))
}

pub async fn delete_group(
    State(state): State<SharedState>,
    tenant: ScimTenant,
    Path((_organization_id, id)): Path<(String, String)>,
) -> ScimResult {
    let org = &tenant.organization_id;
    let deleted = sqlx::query(
        "DELETE FROM rbac_groups WHERE id = ?1 AND organization_id = ?2 AND source = 'scim'",
    )
    .bind(&id)
    .bind(org)
    .execute(&state.db)
    .await
    .map_err(db)?;
    if deleted.rows_affected() == 0 {
        return Err(not_found());
    }
    audit(&state, org, "scim.group_deleted", &id, &id, json!({})).await;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::extract::Request;
    use axum::http::Method;
    use tower::ServiceExt;

    use super::*;
    use crate::testing::{cleanup_database, test_state};

    const TOKEN: &str = "scim-secret";

    async fn setup() -> (SharedState, std::path::PathBuf) {
        let (state, path) = test_state().await;
        for sql in [
            "INSERT INTO sso_domains (id, organization_id, domain, verify_token, verified_at) \
             VALUES ('d1', 'organization-a', 'contoso.com', 't', datetime('now'))",
        ] {
            sqlx::query(sql).execute(&state.db).await.unwrap();
        }
        sqlx::query("UPDATE organizations SET scim_token_digest = ?1 WHERE id = 'organization-a'")
            .bind(security::digest_token(TOKEN))
            .execute(&state.db)
            .await
            .unwrap();
        (state, path)
    }

    async fn call(
        state: &SharedState,
        token: &str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let router = Router::new()
            .nest(
                "/scim/v2/{organization_id}",
                crate::router::router::scim_routes(),
            )
            .with_state(state.clone());
        let request = Request::builder()
            .method(method)
            .uri(format!("/scim/v2/organization-a{path}"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/scim+json")
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .unwrap();
        let response = router.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn count(state: &SharedState, sql: &str, user_id: &str) -> i64 {
        sqlx::query_scalar(sql)
            .bind(user_id)
            .fetch_one(&state.db)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn the_token_is_required() {
        let (state, path) = setup().await;
        let (status, _) = call(&state, "wrong", Method::GET, "/Users", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _) = call(&state, TOKEN, Method::GET, "/ServiceProviderConfig", None).await;
        assert_eq!(status, StatusCode::OK);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn users_are_provisioned_found_and_deprovisioned() {
        let (state, path) = setup().await;
        let (status, body) = call(&state, TOKEN, Method::POST, "/Users", Some(json!({
            "schemas": [USER_SCHEMA], "userName": "Ana@contoso.com", "displayName": "Ana", "active": true,
        }))).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let id = body["id"].as_str().unwrap().to_string();

        let (status, _) = call(
            &state,
            TOKEN,
            Method::POST,
            "/Users",
            Some(json!({"userName": "eve@evil.example"})),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "outside the verified domains"
        );

        let (_, list) = call(
            &state,
            TOKEN,
            Method::GET,
            "/Users?filter=userName%20eq%20%22ana%40contoso.com%22",
            None,
        )
        .await;
        assert_eq!(list["totalResults"], 1);
        let (_, list) = call(
            &state,
            TOKEN,
            Method::GET,
            "/Users?filter=userName%20eq%20%22editor%40example.com%22",
            None,
        )
        .await;
        assert_eq!(list["totalResults"], 1, "existing members are visible");

        // A session and a token that deprovisioning must kill.
        sqlx::query("INSERT INTO sessions (id, user_id, token_digest, expires_at) VALUES ('s', ?1, 'tok', datetime('now', '+1 day'))")
            .bind(&id).execute(&state.db).await.unwrap();
        sqlx::query("INSERT INTO personal_access_tokens (id, user_id, name, token_prefix, token_digest, scopes) VALUES ('p', ?1, 'ci', 'lp_', 'dg', '[\"read\"]')")
            .bind(&id).execute(&state.db).await.unwrap();

        let (status, body) = call(
            &state,
            TOKEN,
            Method::PATCH,
            &format!("/Users/{id}"),
            Some(json!({
                "schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
                "Operations": [{ "op": "Replace", "path": "active", "value": "False" }],
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["active"], false);
        assert_eq!(
            count(
                &state,
                "SELECT COUNT(*) FROM sessions WHERE user_id = ?1",
                &id
            )
            .await,
            0
        );
        assert_eq!(count(&state, "SELECT COUNT(*) FROM personal_access_tokens WHERE user_id = ?1 AND revoked_at IS NULL", &id).await, 0);
        assert_eq!(
            count(
                &state,
                "SELECT COUNT(*) FROM organization_members WHERE user_id = ?1",
                &id
            )
            .await,
            0
        );
        assert_eq!(
            count(
                &state,
                "SELECT COUNT(*) FROM users WHERE id = ?1 AND deactivated_at IS NOT NULL",
                &id
            )
            .await,
            1
        );

        let (_, body) = call(
            &state,
            TOKEN,
            Method::PATCH,
            &format!("/Users/{id}"),
            Some(json!({
                "Operations": [{ "op": "replace", "value": { "active": true } }],
            })),
        )
        .await;
        assert_eq!(body["active"], true, "reactivated");
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn groups_track_their_members() {
        let (state, path) = setup().await;
        let (status, body) = call(
            &state,
            TOKEN,
            Method::POST,
            "/Groups",
            Some(json!({
                "displayName": "Ops", "members": [{ "value": "editor" }, { "value": "outsider" }],
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(
            body["members"].as_array().unwrap().len(),
            1,
            "non-members are skipped"
        );
        let id = body["id"].as_str().unwrap().to_string();

        let (_, body) = call(
            &state,
            TOKEN,
            Method::PATCH,
            &format!("/Groups/{id}"),
            Some(json!({
                "Operations": [
                    { "op": "add", "path": "members", "value": [{ "value": "viewer" }] },
                    { "op": "remove", "path": "members[value eq \"editor\"]" },
                ],
            })),
        )
        .await;
        let members: Vec<&str> = body["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["value"].as_str().unwrap())
            .collect();
        assert_eq!(members, vec!["viewer"]);

        let source: String = sqlx::query_scalar("SELECT source FROM rbac_groups WHERE id = ?1")
            .bind(&id)
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(source, "scim");
        cleanup_database(&path);
    }
}
