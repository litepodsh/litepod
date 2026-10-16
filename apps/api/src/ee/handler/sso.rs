//! Enterprise identity routes (spec 3). Registered only when `!state.is_cloud`
//! (see `router.rs`): on cloud they do not exist.

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect},
};
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::dtos::response::{AppError, created, ok};
use crate::ee::sso::{self, db, dns, oidc};
use crate::git::repositories::audit_log;
use crate::security::authz::extract::{RequireOrg, perm};
use crate::security::middleware::auth::AuthUser;
use crate::security::middleware::org_scope::OrgScope;
use crate::security::middleware::request_context::RequestContext;
use crate::state::state::SharedState;
use crate::utils::cookie::{SSO_REAUTH_COOKIE, add_session_cookie, clear_sso_reauth_cookie};
use crate::utils::security;

async fn audit(
    state: &SharedState,
    org: &OrgScope,
    action: &str,
    id: &str,
    name: &str,
    meta: serde_json::Value,
) {
    audit_log::record(
        state,
        &org.organization_id,
        &org.user,
        action,
        Some("sso"),
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

// ── Login ───────────────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct SsoStartRequest {
    pub email: String,
}

#[derive(Serialize, ToSchema)]
pub struct SsoStartResponse {
    /// Send the browser here.
    pub authorize_url: String,
}

#[utoipa::path(
    post, path = "/api/v1/auth/sso/start", request_body = SsoStartRequest,
    responses(
        (status = 200, description = "Identity provider URL", body = SsoStartResponse),
        (status = 401, description = "No single sign-on for this address"),
    ),
    tag = "auth"
)]
pub async fn start_handler(
    State(state): State<SharedState>,
    jar: CookieJar,
    Json(req): Json<SsoStartRequest>,
) -> Result<impl IntoResponse, AppError> {
    let public = crate::handler::settings::public_base_url(&state).await;
    // Logged out here earlier without ending the IdP session: force credentials.
    let forced = jar.get(SSO_REAUTH_COOKIE).is_some();
    let authorize_url = sso::start_login(&state, &req.email, &public, forced).await?;
    Ok(ok(SsoStartResponse { authorize_url }))
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    /// Set by the IdP when the user cancelled or the IdP refused.
    pub error: Option<String>,
    pub error_description: Option<String>,
}

/// Ends a sign-in: session cookie and the app on success, the login page
/// with the reason otherwise. Shared by the OIDC callback and the SAML ACS.
async fn complete_login(
    state: &SharedState,
    jar: CookieJar,
    headers: &HeaderMap,
    context: Option<&RequestContext>,
    protocol: &str,
    outcome: Result<sso::Finish, AppError>,
) -> (CookieJar, Redirect) {
    let public = crate::handler::settings::public_base_url(state).await;
    let base = public.trim_end_matches('/').to_string();
    let fail = |jar: CookieJar, message: String| {
        (
            jar,
            Redirect::to(&format!(
                "{base}/login?sso_error={}",
                urlencoding::encode(&message)
            )),
        )
    };
    let (user_id, session) = match outcome {
        Ok(sso::Finish::SignedIn { user_id, session }) => (user_id, session),
        // The IdP is asked again with credentials forced (see `sso::gate`).
        Ok(sso::Finish::Reauth { redirect_url }) => return (jar, Redirect::to(&redirect_url)),
        Err(e) => return fail(jar, e.to_string()),
    };
    let user_agent = headers.get("user-agent").and_then(|v| v.to_str().ok());
    let token = match crate::cqrs::commands::auth::create_session(
        state,
        &user_id,
        user_agent,
        Some(&session),
    )
    .await
    {
        Ok(token) => token,
        Err(e) => return fail(jar, e.to_string()),
    };
    if let Some(user) = crate::git::repositories::user::get_user_by_id(state, &user_id).await {
        let actor = AuthUser {
            user_id: user.id.clone(),
            email: user.email.clone(),
            name: user.name.clone(),
            is_sysadmin: false,
            token_id: None,
            token_name: None,
        };
        let _ = crate::security::security_events::record(
            state,
            Some(&actor.user_id),
            &actor,
            "auth.login_sso",
            None,
        )
        .await;
        // 2FA, if any, happened at the IdP, so it is not recorded here.
        let metadata = json!({ "method": "sso", "sso_protocol": protocol }).to_string();
        crate::handler::auth::audit_auth_event(
            state,
            &actor,
            "auth.login",
            Some(&metadata),
            context,
        )
        .await;
    }
    let jar = add_session_cookie(state, jar, token).await;
    // A `Set-Cookie` deletion works even on the SAML cross-site POST, where the
    // Lax cookie itself was not sent.
    let jar = clear_sso_reauth_cookie(state, jar).await;
    (jar, Redirect::to(&format!("{base}/")))
}

/// The IdP redirects the browser here. Success sets the session cookie and
/// lands on the app; any failure lands on the login page with a reason.
#[utoipa::path(
    get, path = "/api/v1/auth/sso/callback",
    responses((status = 302, description = "To the app, or back to /login?sso_error=")),
    tag = "auth"
)]
pub async fn callback_handler(
    State(state): State<SharedState>,
    jar: CookieJar,
    headers: HeaderMap,
    context: Option<Extension<RequestContext>>,
    Query(query): Query<CallbackQuery>,
) -> (CookieJar, Redirect) {
    let outcome = match (query.code, query.state) {
        (Some(code), Some(state_param)) => {
            let public = crate::handler::settings::public_base_url(&state).await;
            oidc::finish(&state, &code, &state_param, &public).await
        }
        _ => Err(AppError::ValidationError(
            query
                .error_description
                .or(query.error)
                .unwrap_or_else(|| "The identity provider did not complete the sign-in".into()),
        )),
    };
    complete_login(&state, jar, &headers, context.as_deref(), "oidc", outcome).await
}

#[derive(Deserialize, ToSchema)]
pub struct BackchannelForm {
    /// The IdP's signed logout token (JWT).
    pub logout_token: String,
}

/// OIDC back-channel logout: the IdP signs a user out of Litepod server to
/// server. Public (no cookie, no Origin); trust comes from the signed token.
#[utoipa::path(
    post, path = "/api/v1/auth/sso/backchannel/{connection_id}",
    responses(
        (status = 200, description = "Sessions of that subject removed (or none existed)"),
        (status = 400, description = "Invalid logout token or connection"),
    ),
    tag = "auth"
)]
pub async fn backchannel_logout_handler(
    State(state): State<SharedState>,
    Path(connection_id): Path<String>,
    axum::extract::Form(form): axum::extract::Form<BackchannelForm>,
) -> axum::response::Response {
    let no_store = [(axum::http::header::CACHE_CONTROL, "no-store")];
    let connection = match state.db.acquire().await {
        Ok(mut conn) => sso::connection(&mut conn, &connection_id)
            .await
            .ok()
            .flatten(),
        Err(_) => None,
    }
    .filter(|c| c.enabled && c.kind == "oidc");
    let Some(connection) = connection else {
        return (StatusCode::BAD_REQUEST, no_store).into_response();
    };
    let Ok(subject) = oidc::validate_logout_token(&state, &connection, &form.logout_token).await
    else {
        return (StatusCode::BAD_REQUEST, no_store).into_response();
    };
    if let Err(error) =
        crate::git::repositories::session::revoke_for_subject(&state, &connection.id, &subject)
            .await
    {
        tracing::error!(%error, "sso: back-channel logout could not delete sessions");
        return (StatusCode::INTERNAL_SERVER_ERROR, no_store).into_response();
    }
    audit_backchannel(&state, &connection.id, &subject).await;
    (StatusCode::OK, no_store).into_response()
}

async fn audit_backchannel(state: &SharedState, connection_id: &str, subject: &str) {
    let user_id: Option<String> = sqlx::query_scalar(
        "SELECT user_id FROM user_identities WHERE connection_id = ?1 AND subject = ?2",
    )
    .bind(connection_id)
    .bind(subject)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some(user_id) = user_id else { return };
    let Some(user) = crate::git::repositories::user::get_user_by_id(state, &user_id).await else {
        return;
    };
    let actor = AuthUser {
        user_id: user.id.clone(),
        email: user.email.clone(),
        name: user.name.clone(),
        is_sysadmin: false,
        token_id: None,
        token_name: None,
    };
    let metadata = json!({ "method": "sso_backchannel" }).to_string();
    crate::handler::auth::audit_auth_event(state, &actor, "auth.logout", Some(&metadata), None)
        .await;
}

#[cfg(feature = "saml")]
#[derive(Deserialize)]
pub struct AcsForm {
    #[serde(rename = "SAMLResponse")]
    pub saml_response: String,
    #[serde(rename = "RelayState", default)]
    pub relay_state: String,
}

/// SAML Assertion Consumer Service (HTTP-POST binding).
#[cfg(feature = "saml")]
pub async fn saml_acs_handler(
    State(state): State<SharedState>,
    jar: CookieJar,
    headers: HeaderMap,
    context: Option<Extension<RequestContext>>,
    axum::extract::Form(form): axum::extract::Form<AcsForm>,
) -> (CookieJar, Redirect) {
    let public = crate::handler::settings::public_base_url(&state).await;
    let outcome = sso::saml::finish(&state, &form.saml_response, &form.relay_state, &public).await;
    complete_login(&state, jar, &headers, context.as_deref(), "saml", outcome).await
}

/// SP metadata for a SAML connection: give this URL (or file) to the IdP.
#[cfg(feature = "saml")]
pub async fn saml_metadata_handler(
    State(state): State<SharedState>,
    Path(connection_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let mut conn = state.db.acquire().await.map_err(db)?;
    let connection = sso::connection(&mut conn, &connection_id)
        .await?
        .filter(|c| c.kind == "saml")
        .ok_or(AppError::ResourceNotFound)?;
    let public = crate::handler::settings::public_base_url(&state).await;
    let xml = sso::saml::sp_metadata(&connection, &public)?;
    Ok((
        [(
            axum::http::header::CONTENT_TYPE,
            "application/samlmetadata+xml",
        )],
        xml,
    ))
}

// ── Connections ─────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct ConnectionDto {
    pub id: String,
    /// `oidc` or `saml`.
    pub kind: String,
    pub name: String,
    pub enabled: bool,
    pub issuer: Option<String>,
    pub client_id: Option<String>,
    /// The secret itself is write-only.
    pub has_client_secret: bool,
    /// SAML: the IdP entity id read from its metadata.
    pub idp_entity_id: Option<String>,
    pub groups_claim: String,
    pub jit_enabled: bool,
    pub default_base_role_id: i64,
    /// Paste into the IdP: OIDC redirect URI, or SAML ACS URL.
    pub redirect_uri: String,
    /// SAML: the SP entity id, which also serves the SP metadata.
    pub sp_entity_id: Option<String>,
    /// Minutes a Litepod session from this connection lasts.
    pub session_ttl_minutes: i64,
    /// OIDC: paste into the IdP's allowed post-logout redirect URIs.
    pub post_logout_redirect_uri: Option<String>,
    /// OIDC: paste into the IdP's back-channel logout URL.
    pub backchannel_logout_url: Option<String>,
}

fn to_dto(c: sso::Connection, public_base_url: &str) -> ConnectionDto {
    let saml = c.kind == "saml";
    ConnectionDto {
        redirect_uri: if saml {
            format!(
                "{}/api/v1/auth/saml/acs",
                public_base_url.trim_end_matches('/')
            )
        } else {
            oidc::redirect_uri(public_base_url)
        },
        sp_entity_id: saml.then(|| {
            format!(
                "{}/api/v1/auth/saml/{}/metadata",
                public_base_url.trim_end_matches('/'),
                c.id
            )
        }),
        post_logout_redirect_uri: (!saml)
            .then(|| format!("{}/login", public_base_url.trim_end_matches('/'))),
        backchannel_logout_url: (!saml).then(|| {
            format!(
                "{}/api/v1/auth/sso/backchannel/{}",
                public_base_url.trim_end_matches('/'),
                c.id
            )
        }),
        session_ttl_minutes: c.session_ttl_minutes,
        id: c.id,
        kind: c.kind,
        name: c.name,
        enabled: c.enabled,
        issuer: c.issuer,
        client_id: c.client_id,
        has_client_secret: c.client_secret_encrypted.is_some(),
        idp_entity_id: c.idp_entity_id,
        groups_claim: c.groups_claim,
        jit_enabled: c.jit_enabled,
        default_base_role_id: c.default_base_role_id,
    }
}

#[derive(Deserialize, ToSchema)]
pub struct ConnectionInput {
    /// `oidc` (default) or `saml`; fixed once created.
    #[serde(default = "oidc_kind")]
    pub kind: String,
    pub name: String,
    /// OIDC only.
    #[serde(default)]
    pub issuer: String,
    /// OIDC only.
    #[serde(default)]
    pub client_id: String,
    /// OIDC only. Omit (or null) to keep the stored secret.
    pub client_secret: Option<String>,
    /// SAML only: the IdP's federation metadata XML. Omit to keep it.
    pub idp_metadata_xml: Option<String>,
    #[serde(default = "default_groups_claim")]
    pub groups_claim: String,
    #[serde(default = "yes")]
    pub jit_enabled: bool,
    #[serde(default = "member")]
    pub default_base_role_id: i64,
    #[serde(default = "three_days")]
    pub session_ttl_minutes: i64,
    #[serde(default)]
    pub enabled: bool,
}

fn oidc_kind() -> String {
    "oidc".into()
}
fn default_groups_claim() -> String {
    "groups".into()
}
fn yes() -> bool {
    true
}
fn member() -> i64 {
    4
}
fn three_days() -> i64 {
    4320
}

/// OIDC: HTTPS issuer (plain HTTP only in dev mode, for a local Keycloak).
/// Private addresses are allowed: an IdP on the internal network is the normal
/// self-hosted setup, and only org SSO managers can set this.
/// SAML: metadata must parse and name a signing certificate.
/// Returns the SAML IdP entity id when new metadata was given.
fn validate(
    state: &SharedState,
    kind: &str,
    input: &ConnectionInput,
) -> Result<Option<String>, AppError> {
    if input.name.trim().is_empty() {
        return Err(AppError::ValidationError("Name is required".into()));
    }
    if !(1..=4).contains(&input.default_base_role_id) {
        return Err(AppError::ValidationError(
            "default_base_role_id must be 1 to 4".into(),
        ));
    }
    if !(5..=43200).contains(&input.session_ttl_minutes) {
        return Err(AppError::ValidationError(
            "session_ttl_minutes must be 5 to 43200".into(),
        ));
    }
    if input.groups_claim.trim().is_empty() {
        return Err(AppError::ValidationError("Groups claim is required".into()));
    }
    match kind {
        "oidc" => {
            if input.client_id.trim().is_empty() {
                return Err(AppError::ValidationError("Client ID is required".into()));
            }
            let url = url::Url::parse(input.issuer.trim())
                .map_err(|_| AppError::ValidationError("Issuer must be a URL".into()))?;
            if url.scheme() != "https" && !(state.dev_mode && url.scheme() == "http") {
                return Err(AppError::ValidationError("Issuer must use https".into()));
            }
            Ok(None)
        }
        "saml" if cfg!(feature = "saml") => match input.idp_metadata_xml.as_deref() {
            #[cfg(feature = "saml")]
            Some(xml) if !xml.trim().is_empty() => sso::saml::validate_metadata(xml).map(Some),
            _ => Ok(None),
        },
        "saml" => Err(AppError::ValidationError(
            "This Litepod build does not include SAML (cargo feature `saml`)".into(),
        )),
        _ => Err(AppError::ValidationError(
            "kind must be oidc or saml".into(),
        )),
    }
}

async fn ensure_can_enable(
    conn: &mut sqlx::SqliteConnection,
    organization_id: &str,
) -> Result<(), AppError> {
    let verified: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sso_domains WHERE organization_id = ?1 AND verified_at IS NOT NULL",
    )
    .bind(organization_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(db)?;
    if verified == 0 {
        return Err(AppError::ResourceInUse(
            "Verify at least one domain before enabling single sign-on".into(),
        ));
    }
    Ok(())
}

async fn owned_connection(
    conn: &mut sqlx::SqliteConnection,
    organization_id: &str,
    connection_id: &str,
) -> Result<sso::Connection, AppError> {
    sso::connection(conn, connection_id)
        .await?
        .filter(|c| c.organization_id == organization_id)
        .ok_or(AppError::ResourceNotFound)
}

#[utoipa::path(
    get, path = "/api/v1/organizations/{organization_id}/sso/connections",
    responses((status = 200, description = "SSO connections", body = [ConnectionDto])),
    tag = "sso"
)]
pub async fn list_connections_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgSsoManage>,
) -> Result<impl IntoResponse, AppError> {
    let public = crate::handler::settings::public_base_url(&state).await;
    let mut conn = state.db.acquire().await.map_err(db)?;
    let list: Vec<ConnectionDto> = sso::connections_of(&mut conn, &org.organization_id)
        .await?
        .into_iter()
        .map(|c| to_dto(c, &public))
        .collect();
    Ok(ok(list))
}

/// OIDC fields are stored only for OIDC, SAML fields only for SAML.
fn oidc_field(kind: &str, value: &str) -> Option<String> {
    (kind == "oidc").then(|| value.trim().to_string())
}

#[utoipa::path(
    post, path = "/api/v1/organizations/{organization_id}/sso/connections",
    request_body = ConnectionInput,
    responses((status = 201, description = "Connection created", body = ConnectionDto)),
    tag = "sso"
)]
pub async fn create_connection_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgSsoManage>,
    Json(input): Json<ConnectionInput>,
) -> Result<impl IntoResponse, AppError> {
    let kind = input.kind.clone();
    let idp_entity_id = validate(&state, &kind, &input)?;
    if kind == "saml" && idp_entity_id.is_none() {
        return Err(AppError::ValidationError(
            "IdP metadata XML is required".into(),
        ));
    }
    let mut conn = state.db.acquire().await.map_err(db)?;
    if input.enabled {
        ensure_can_enable(&mut conn, &org.organization_id).await?;
    }
    let secret = if kind == "oidc" {
        encrypt_secret(input.client_secret.as_deref())?
    } else {
        None
    };
    let id = Uuid::now_v7().to_string();
    sqlx::query(
        "INSERT INTO sso_connections
            (id, organization_id, kind, name, enabled, issuer, client_id, client_secret_encrypted,
             groups_claim, jit_enabled, default_base_role_id, idp_metadata_xml, idp_entity_id,
             session_ttl_minutes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
    )
    .bind(&id)
    .bind(&org.organization_id)
    .bind(&kind)
    .bind(input.name.trim())
    .bind(input.enabled as i64)
    .bind(oidc_field(&kind, &input.issuer))
    .bind(oidc_field(&kind, &input.client_id))
    .bind(secret)
    .bind(input.groups_claim.trim())
    .bind(input.jit_enabled as i64)
    .bind(input.default_base_role_id)
    .bind(
        idp_entity_id
            .as_ref()
            .and(input.idp_metadata_xml.as_deref()),
    )
    .bind(&idp_entity_id)
    .bind(input.session_ttl_minutes)
    .execute(&mut *conn)
    .await
    .map_err(db)?;
    let created_row = owned_connection(&mut conn, &org.organization_id, &id).await?;
    audit(
        &state,
        &org,
        "sso.connection_created",
        &id,
        &created_row.name,
        json!({ "kind": created_row.kind, "issuer": created_row.issuer, "idp_entity_id": created_row.idp_entity_id }),
    )
    .await;
    let public = crate::handler::settings::public_base_url(&state).await;
    Ok(created(to_dto(created_row, &public)))
}

fn encrypt_secret(secret: Option<&str>) -> Result<Option<String>, AppError> {
    secret
        .filter(|s| !s.trim().is_empty())
        .map(|s| {
            security::encrypt_data(s.trim(), &security::get_encryption_key())
                .map_err(AppError::InternalError)
        })
        .transpose()
}

#[utoipa::path(
    patch, path = "/api/v1/organizations/{organization_id}/sso/connections/{connection_id}",
    request_body = ConnectionInput,
    responses(
        (status = 200, description = "Connection updated", body = ConnectionDto),
        (status = 409, description = "Enabling without a verified domain"),
    ),
    tag = "sso"
)]
pub async fn update_connection_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgSsoManage>,
    Path((_organization_id, connection_id)): Path<(String, String)>,
    Json(input): Json<ConnectionInput>,
) -> Result<impl IntoResponse, AppError> {
    let mut conn = state.db.acquire().await.map_err(db)?;
    let current = owned_connection(&mut conn, &org.organization_id, &connection_id).await?;
    // The kind is fixed at creation; the stored one decides what to validate.
    let kind = current.kind.clone();
    let new_entity_id = validate(&state, &kind, &input)?;
    if input.enabled {
        ensure_can_enable(&mut conn, &org.organization_id).await?;
    }
    let secret = if kind == "oidc" {
        encrypt_secret(input.client_secret.as_deref())?.or(current.client_secret_encrypted)
    } else {
        None
    };
    let (metadata, entity_id) = match new_entity_id {
        Some(entity_id) => (input.idp_metadata_xml.clone(), Some(entity_id)),
        None => (current.idp_metadata_xml, current.idp_entity_id),
    };
    sqlx::query(
        "UPDATE sso_connections SET name = ?1, enabled = ?2, issuer = ?3, client_id = ?4,
                client_secret_encrypted = ?5, groups_claim = ?6, jit_enabled = ?7,
                default_base_role_id = ?8, idp_metadata_xml = ?9, idp_entity_id = ?10,
                session_ttl_minutes = ?11
          WHERE id = ?12",
    )
    .bind(input.name.trim())
    .bind(input.enabled as i64)
    .bind(oidc_field(&kind, &input.issuer))
    .bind(oidc_field(&kind, &input.client_id))
    .bind(secret)
    .bind(input.groups_claim.trim())
    .bind(input.jit_enabled as i64)
    .bind(input.default_base_role_id)
    .bind(metadata)
    .bind(entity_id)
    .bind(input.session_ttl_minutes)
    .bind(&connection_id)
    .execute(&mut *conn)
    .await
    .map_err(db)?;
    let updated = owned_connection(&mut conn, &org.organization_id, &connection_id).await?;
    audit(
        &state,
        &org,
        "sso.connection_updated",
        &connection_id,
        &updated.name,
        json!({
            "enabled": updated.enabled,
            "issuer": updated.issuer,
            "idp_entity_id": updated.idp_entity_id,
            "secret_changed": input.client_secret.is_some(),
            "metadata_changed": input.idp_metadata_xml.is_some(),
        }),
    )
    .await;
    let public = crate::handler::settings::public_base_url(&state).await;
    Ok(ok(to_dto(updated, &public)))
}

#[utoipa::path(
    delete, path = "/api/v1/organizations/{organization_id}/sso/connections/{connection_id}",
    responses((status = 204, description = "Connection, its identities and mappings deleted")),
    tag = "sso"
)]
pub async fn delete_connection_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgSsoManage>,
    Path((_organization_id, connection_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let mut conn = state.db.acquire().await.map_err(db)?;
    let current = owned_connection(&mut conn, &org.organization_id, &connection_id).await?;
    sqlx::query("DELETE FROM sso_connections WHERE id = ?1")
        .bind(&connection_id)
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    audit(
        &state,
        &org,
        "sso.connection_deleted",
        &connection_id,
        &current.name,
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

// ── Group mappings ──────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, ToSchema)]
pub struct GroupMappingDto {
    /// IdP group id (Entra objectId).
    pub external_group: String,
    /// Litepod group id; it becomes IdP-managed.
    pub group_id: String,
}

#[utoipa::path(
    get, path = "/api/v1/organizations/{organization_id}/sso/connections/{connection_id}/group-mappings",
    responses((status = 200, description = "IdP group → Litepod group", body = [GroupMappingDto])),
    tag = "sso"
)]
pub async fn list_mappings_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgSsoManage>,
    Path((_organization_id, connection_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let mut conn = state.db.acquire().await.map_err(db)?;
    owned_connection(&mut conn, &org.organization_id, &connection_id).await?;
    let rows = sqlx::query(
        "SELECT external_group, group_id FROM sso_group_mappings WHERE connection_id = ?1 ORDER BY external_group",
    )
    .bind(&connection_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(db)?;
    let list: Vec<GroupMappingDto> = rows
        .iter()
        .map(|r| GroupMappingDto {
            external_group: r.get(0),
            group_id: r.get(1),
        })
        .collect();
    Ok(ok(list))
}

/// Replaces the mappings. Mapped groups switch to `source = 'idp'`, so their
/// membership is no longer edited by hand: each login sets it.
#[utoipa::path(
    put, path = "/api/v1/organizations/{organization_id}/sso/connections/{connection_id}/group-mappings",
    request_body = [GroupMappingDto],
    responses((status = 200, description = "Mappings replaced", body = [GroupMappingDto])),
    tag = "sso"
)]
pub async fn set_mappings_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgSsoManage>,
    Path((_organization_id, connection_id)): Path<(String, String)>,
    Json(mappings): Json<Vec<GroupMappingDto>>,
) -> Result<impl IntoResponse, AppError> {
    let mut tx = state.db.begin().await.map_err(db)?;
    let current = owned_connection(&mut tx, &org.organization_id, &connection_id).await?;
    sqlx::query("DELETE FROM sso_group_mappings WHERE connection_id = ?1")
        .bind(&connection_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    for m in &mappings {
        if m.external_group.trim().is_empty() {
            return Err(AppError::ValidationError("IdP group id is required".into()));
        }
        // SCIM groups belong to SCIM; local and IdP groups can be mapped.
        let source: Option<String> = sqlx::query_scalar(
            "SELECT source FROM rbac_groups WHERE id = ?1 AND organization_id = ?2",
        )
        .bind(&m.group_id)
        .bind(&org.organization_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db)?;
        match source.as_deref() {
            None => {
                return Err(AppError::ValidationError(format!(
                    "Unknown group {}",
                    m.group_id
                )));
            }
            Some("scim") => {
                return Err(AppError::ValidationError(
                    "SCIM groups are managed by SCIM and cannot be mapped".into(),
                ));
            }
            _ => {}
        }
        sqlx::query(
            "INSERT INTO sso_group_mappings (connection_id, external_group, group_id) VALUES (?1, ?2, ?3)",
        )
        .bind(&connection_id)
        .bind(m.external_group.trim())
        .bind(&m.group_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
        sqlx::query("UPDATE rbac_groups SET source = 'idp' WHERE id = ?1")
            .bind(&m.group_id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
    }
    tx.commit().await.map_err(db)?;
    audit(
        &state,
        &org,
        "sso.group_mappings_changed",
        &connection_id,
        &current.name,
        json!({ "mappings": mappings.iter().map(|m| (&m.external_group, &m.group_id)).collect::<Vec<_>>() }),
    )
    .await;
    Ok(ok(mappings))
}

// ── Domains ─────────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct DomainDto {
    pub id: String,
    pub domain: String,
    pub verified: bool,
    /// The record's name relative to the domain, for DNS panels' "Host" field.
    pub txt_host: String,
    /// Publish a TXT record with this name…
    pub txt_name: String,
    /// …and this value.
    pub txt_value: String,
    /// The connection that serves this domain; null = the oldest enabled one.
    pub connection_id: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct DomainInput {
    pub domain: String,
}

fn domain_dto(row: &sqlx::sqlite::SqliteRow) -> DomainDto {
    let domain: String = row.get(1);
    DomainDto {
        id: row.get(0),
        txt_host: dns::RECORD_HOST.to_string(),
        txt_name: dns::record_name(&domain),
        domain,
        verified: row.get::<Option<String>, _>(2).is_some(),
        txt_value: row.get(3),
        connection_id: row.get(4),
    }
}

fn normalize_domain(raw: &str) -> Result<String, AppError> {
    let domain = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    let labels: Vec<&str> = domain.split('.').collect();
    let valid = labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty()
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    if !valid {
        return Err(AppError::ValidationError(
            "Enter a domain like company.com".into(),
        ));
    }
    Ok(domain)
}

#[utoipa::path(
    get, path = "/api/v1/organizations/{organization_id}/sso/domains",
    responses((status = 200, description = "Claimed domains", body = [DomainDto])),
    tag = "sso"
)]
pub async fn list_domains_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgSsoManage>,
) -> Result<impl IntoResponse, AppError> {
    let rows = sqlx::query(
        "SELECT id, domain, verified_at, verify_token, connection_id FROM sso_domains WHERE organization_id = ?1 ORDER BY domain",
    )
    .bind(&org.organization_id)
    .fetch_all(&state.db)
    .await
    .map_err(db)?;
    let list: Vec<DomainDto> = rows.iter().map(domain_dto).collect();
    Ok(ok(list))
}

#[utoipa::path(
    post, path = "/api/v1/organizations/{organization_id}/sso/domains",
    request_body = DomainInput,
    responses(
        (status = 201, description = "Domain claimed, unverified", body = DomainDto),
        (status = 409, description = "Already claimed (by this or another organization)"),
    ),
    tag = "sso"
)]
pub async fn create_domain_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgSsoManage>,
    Json(input): Json<DomainInput>,
) -> Result<impl IntoResponse, AppError> {
    let domain = normalize_domain(&input.domain)?;
    let id = Uuid::now_v7().to_string();
    let token = format!("litepod-verify={}", security::generate_token());
    sqlx::query(
        "INSERT INTO sso_domains (id, organization_id, domain, verify_token) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(&id)
    .bind(&org.organization_id)
    .bind(&domain)
    .bind(&token)
    .execute(&state.db)
    .await
    .map_err(db)?;
    audit(&state, &org, "sso.domain_added", &id, &domain, json!({})).await;
    Ok(created(DomainDto {
        id,
        txt_host: dns::RECORD_HOST.to_string(),
        txt_name: dns::record_name(&domain),
        domain,
        verified: false,
        txt_value: token,
        connection_id: None,
    }))
}

#[utoipa::path(
    post, path = "/api/v1/organizations/{organization_id}/sso/domains/{domain_id}/verify",
    responses(
        (status = 200, description = "Verified", body = DomainDto),
        (status = 422, description = "TXT record not found yet"),
    ),
    tag = "sso"
)]
pub async fn verify_domain_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgSsoManage>,
    Path((_organization_id, domain_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let row = sqlx::query(
        "SELECT id, domain, verified_at, verify_token, connection_id FROM sso_domains WHERE id = ?1 AND organization_id = ?2",
    )
    .bind(&domain_id)
    .bind(&org.organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(db)?
    .ok_or(AppError::ResourceNotFound)?;
    let mut dto = domain_dto(&row);
    if !dto.verified {
        let mut found = false;
        for name in dns::candidate_names(&dto.domain) {
            if dns::txt_values(&name)
                .await
                .iter()
                .any(|v| v.trim() == dto.txt_value)
            {
                found = true;
                break;
            }
        }
        if !found {
            return Err(AppError::ValidationError(format!(
                "TXT record {} not found yet (DNS changes can take a few minutes)",
                dto.txt_name
            )));
        }
        sqlx::query("UPDATE sso_domains SET verified_at = datetime('now') WHERE id = ?1")
            .bind(&domain_id)
            .execute(&state.db)
            .await
            .map_err(db)?;
        dto.verified = true;
        audit(
            &state,
            &org,
            "sso.domain_verified",
            &domain_id,
            &dto.domain,
            json!({}),
        )
        .await;
    }
    Ok(ok(dto))
}

#[derive(Deserialize, ToSchema)]
pub struct DomainConnectionInput {
    /// Null = the organization's oldest enabled connection.
    pub connection_id: Option<String>,
}

#[utoipa::path(
    patch, path = "/api/v1/organizations/{organization_id}/sso/domains/{domain_id}",
    request_body = DomainConnectionInput,
    responses(
        (status = 200, description = "Domain assigned", body = DomainDto),
        (status = 404, description = "Domain or connection not in this organization"),
    ),
    tag = "sso"
)]
pub async fn assign_domain_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgSsoManage>,
    Path((_organization_id, domain_id)): Path<(String, String)>,
    Json(input): Json<DomainConnectionInput>,
) -> Result<impl IntoResponse, AppError> {
    let mut conn = state.db.acquire().await.map_err(db)?;
    if let Some(connection_id) = &input.connection_id {
        owned_connection(&mut conn, &org.organization_id, connection_id).await?;
    }
    let row = sqlx::query(
        "UPDATE sso_domains SET connection_id = ?1 WHERE id = ?2 AND organization_id = ?3
         RETURNING id, domain, verified_at, verify_token, connection_id",
    )
    .bind(&input.connection_id)
    .bind(&domain_id)
    .bind(&org.organization_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(db)?
    .ok_or(AppError::ResourceNotFound)?;
    let dto = domain_dto(&row);
    audit(
        &state,
        &org,
        "sso.domain_assigned",
        &domain_id,
        &dto.domain,
        json!({ "connection_id": dto.connection_id }),
    )
    .await;
    Ok(ok(dto))
}

#[utoipa::path(
    delete, path = "/api/v1/organizations/{organization_id}/sso/domains/{domain_id}",
    responses(
        (status = 204, description = "Domain released"),
        (status = 409, description = "Last verified domain of an enabled connection"),
    ),
    tag = "sso"
)]
pub async fn delete_domain_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgSsoManage>,
    Path((_organization_id, domain_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let mut tx = state.db.begin().await.map_err(db)?;
    let domain: String =
        sqlx::query_scalar("SELECT domain FROM sso_domains WHERE id = ?1 AND organization_id = ?2")
            .bind(&domain_id)
            .bind(&org.organization_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?
            .ok_or(AppError::ResourceNotFound)?;
    sqlx::query("DELETE FROM sso_domains WHERE id = ?1")
        .bind(&domain_id)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    let enabled: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sso_connections WHERE organization_id = ?1 AND enabled = 1",
    )
    .bind(&org.organization_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(db)?;
    if enabled > 0 {
        ensure_can_enable(&mut tx, &org.organization_id).await?;
    }
    tx.commit().await.map_err(db)?;
    audit(
        &state,
        &org,
        "sso.domain_removed",
        &domain_id,
        &domain,
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

// ── SCIM token and enforcement ──────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct ScimTokenDto {
    /// Shown once; only its digest is stored.
    pub token: String,
    /// Tenant URL for the IdP's provisioning settings.
    pub base_url: String,
}

#[utoipa::path(
    post, path = "/api/v1/organizations/{organization_id}/sso/scim-token",
    responses((status = 200, description = "New SCIM token (replaces the previous one)", body = ScimTokenDto)),
    tag = "sso"
)]
pub async fn rotate_scim_token_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgSsoManage>,
) -> Result<impl IntoResponse, AppError> {
    let token = security::generate_token();
    sqlx::query("UPDATE organizations SET scim_token_digest = ?1 WHERE id = ?2")
        .bind(security::digest_token(&token))
        .bind(&org.organization_id)
        .execute(&state.db)
        .await
        .map_err(db)?;
    audit(
        &state,
        &org,
        "sso.scim_token_rotated",
        &org.organization_id,
        "scim",
        json!({}),
    )
    .await;
    let public = crate::handler::settings::public_base_url(&state).await;
    Ok(ok(ScimTokenDto {
        token,
        base_url: format!(
            "{}/scim/v2/{}",
            public.trim_end_matches('/'),
            org.organization_id
        ),
    }))
}

#[utoipa::path(
    delete, path = "/api/v1/organizations/{organization_id}/sso/scim-token",
    responses((status = 204, description = "SCIM turned off")),
    tag = "sso"
)]
pub async fn delete_scim_token_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgSsoManage>,
) -> Result<impl IntoResponse, AppError> {
    sqlx::query("UPDATE organizations SET scim_token_digest = NULL WHERE id = ?1")
        .bind(&org.organization_id)
        .execute(&state.db)
        .await
        .map_err(db)?;
    audit(
        &state,
        &org,
        "sso.scim_disabled",
        &org.organization_id,
        "scim",
        json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct SsoSettingsDto {
    pub enforced: bool,
    pub scim_enabled: bool,
    /// Whether this build includes SAML (cargo feature `saml`).
    #[serde(default)]
    pub saml_supported: bool,
}

#[utoipa::path(
    get, path = "/api/v1/organizations/{organization_id}/sso/settings",
    responses((status = 200, description = "Enforcement and SCIM state", body = SsoSettingsDto)),
    tag = "sso"
)]
pub async fn get_settings_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgSsoManage>,
) -> Result<impl IntoResponse, AppError> {
    let row =
        sqlx::query("SELECT sso_enforced, scim_token_digest FROM organizations WHERE id = ?1")
            .bind(&org.organization_id)
            .fetch_one(&state.db)
            .await
            .map_err(db)?;
    Ok(ok(SsoSettingsDto {
        enforced: row.get::<i64, _>(0) == 1,
        scim_enabled: row.get::<Option<String>, _>(1).is_some(),
        saml_supported: cfg!(feature = "saml"),
    }))
}

#[derive(Deserialize, ToSchema)]
pub struct EnforcementInput {
    pub enforced: bool,
}

/// Spec §5. Turning enforcement on needs an enabled connection and proof the
/// caller can get back in through it (they signed in with SSO here), so a
/// broken configuration cannot lock the organization out. The SysAdmin can
/// always turn it off (break-glass).
#[utoipa::path(
    put, path = "/api/v1/organizations/{organization_id}/sso/enforcement",
    request_body = EnforcementInput,
    responses(
        (status = 200, description = "Saved", body = SsoSettingsDto),
        (status = 409, description = "No enabled connection, or the caller never signed in through it"),
    ),
    tag = "sso"
)]
pub async fn set_enforcement_handler(
    State(state): State<SharedState>,
    RequireOrg(org, _): RequireOrg<perm::OrgSsoManage>,
    Json(input): Json<EnforcementInput>,
) -> Result<impl IntoResponse, AppError> {
    if input.enforced {
        let proven: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM user_identities i JOIN sso_connections c ON c.id = i.connection_id
              WHERE c.organization_id = ?1 AND c.enabled = 1 AND i.user_id = ?2 LIMIT 1",
        )
        .bind(&org.organization_id)
        .bind(&org.user.user_id)
        .fetch_optional(&state.db)
        .await
        .map_err(db)?;
        if proven.is_none() {
            return Err(AppError::ResourceInUse(
                "Sign in once with single sign-on through an enabled connection before enforcing it".into(),
            ));
        }
    }
    sqlx::query("UPDATE organizations SET sso_enforced = ?1 WHERE id = ?2")
        .bind(input.enforced as i64)
        .bind(&org.organization_id)
        .execute(&state.db)
        .await
        .map_err(db)?;
    audit(
        &state,
        &org,
        "sso.enforcement_changed",
        &org.organization_id,
        "sso",
        json!({ "enforced": input.enforced }),
    )
    .await;
    get_settings_handler(State(state), RequireOrg(org, std::marker::PhantomData)).await
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::extract::Request;
    use axum::http::Method;
    use axum::middleware::Next;
    use axum::routing::{get, post, put};
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::testing::{auth_user, cleanup_database, set_base_role, test_state, test_state_for};

    fn router(state: SharedState, user: AuthUser) -> Router {
        let base = "/api/v1/organizations/{organization_id}/sso";
        Router::new()
            .route(
                &format!("{base}/connections"),
                get(list_connections_handler).post(create_connection_handler),
            )
            .route(
                &format!("{base}/domains"),
                get(list_domains_handler).post(create_domain_handler),
            )
            .route(&format!("{base}/enforcement"), put(set_enforcement_handler))
            .route(
                &format!("{base}/scim-token"),
                post(rotate_scim_token_handler),
            )
            .layer(axum::middleware::from_fn(
                move |mut req: Request, next: Next| {
                    let user = user.clone();
                    async move {
                        req.extensions_mut().insert(user);
                        next.run(req).await
                    }
                },
            ))
            .with_state(state)
    }

    async fn call(
        state: &SharedState,
        org: &str,
        method: Method,
        path: &str,
        body: Value,
    ) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(format!("/api/v1/organizations/{org}/sso{path}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = router(state.clone(), auth_user("editor"))
            .oneshot(request)
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn connection(enabled: bool) -> Value {
        json!({ "name": "Entra", "issuer": "https://login.microsoftonline.com/t/v2.0",
                "client_id": "app", "client_secret": "s3cret", "enabled": enabled })
    }

    /// `editor` is Admin of both organizations.
    async fn setup() -> (SharedState, std::path::PathBuf) {
        let (state, path) = test_state().await;
        set_base_role(&state, "organization-a", "editor", 1).await;
        set_base_role(&state, "organization-b", "editor", 1).await;
        (state, path)
    }

    #[tokio::test]
    async fn backchannel_logout_deletes_only_that_subjects_sessions() {
        let (state, path, idp) = crate::ee::sso::oidc::tests::setup().await;
        let mk = |subject: &str| crate::ee::sso::SsoSession {
            connection_id: "c1".into(),
            subject: subject.into(),
            ttl_minutes: 60,
            id_token_encrypted: None,
        };
        let create = crate::cqrs::commands::auth::create_session;
        let ana = create(&state, "editor", None, Some(&mk("sub-ana")))
            .await
            .unwrap();
        let bob = create(&state, "viewer", None, Some(&mk("sub-bob")))
            .await
            .unwrap();

        let app = Router::new()
            .merge(crate::router::router::sso_routes_for_edition(state.clone()))
            .with_state(state.clone());
        let post_token = |token: String| {
            let app = app.clone();
            async move {
                app.oneshot(
                    Request::post("/api/v1/auth/sso/backchannel/c1")
                        .header("content-type", "application/x-www-form-urlencoded")
                        .body(Body::from(format!(
                            "logout_token={}",
                            urlencoding::encode(&token)
                        )))
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
            }
        };
        assert_eq!(post_token("garbage".into()).await, StatusCode::BAD_REQUEST);
        assert_eq!(
            post_token(idp.sign(&idp.logout_claims(json!({})))).await,
            StatusCode::OK
        );

        use crate::git::repositories::session::get_session_by_token_result as get;
        assert!(get(&state, &ana).await.unwrap().is_none(), "ana signed out");
        assert!(get(&state, &bob).await.unwrap().is_some(), "bob untouched");
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn the_reauth_cookie_forces_credentials_at_start() {
        let (state, path, _idp) = crate::ee::sso::oidc::tests::setup().await;
        // The redirect URI is built from the public URL, empty in tests.
        sqlx::query(
            "INSERT INTO server_domain_settings (id, domain, email, https, updated_at) \
             VALUES (1, 'litepod.test', 'a@litepod.test', 0, 0)",
        )
        .execute(&state.db)
        .await
        .unwrap();
        let app = Router::new()
            .route("/api/v1/auth/sso/start", post(start_handler))
            .with_state(state.clone());
        let start = |cookie: Option<&'static str>| {
            let app = app.clone();
            async move {
                let mut request = Request::post("/api/v1/auth/sso/start")
                    .header("content-type", "application/json");
                if let Some(c) = cookie {
                    request = request.header("cookie", c);
                }
                let response = app
                    .oneshot(
                        request
                            .body(Body::from(r#"{"email":"ana@contoso.com"}"#))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
                let body: Value = serde_json::from_slice(&bytes).unwrap();
                body["data"]["authorize_url"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{body}"))
                    .to_string()
            }
        };
        assert!(!start(None).await.contains("prompt=login"));
        assert!(
            start(Some("lp_sso_reauth=1"))
                .await
                .contains("prompt=login")
        );
        let forced: i64 = sqlx::query_scalar("SELECT MAX(forced) FROM sso_login_states")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(forced, 1);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn session_ttl_is_validated_and_returned() {
        let (state, path) = setup().await;
        let mut body = connection(false);
        body["session_ttl_minutes"] = json!(2);
        let (status, _) = call(
            &state,
            "organization-a",
            Method::POST,
            "/connections",
            body.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "below 5 minutes");

        body["session_ttl_minutes"] = json!(60);
        let (status, created) =
            call(&state, "organization-a", Method::POST, "/connections", body).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(created["data"]["session_ttl_minutes"], 60);
        assert!(
            created["data"]["post_logout_redirect_uri"]
                .as_str()
                .unwrap()
                .ends_with("/login")
        );
        assert!(
            created["data"]["backchannel_logout_url"]
                .as_str()
                .unwrap()
                .contains("/api/v1/auth/sso/backchannel/")
        );

        let (_, created) = call(
            &state,
            "organization-a",
            Method::POST,
            "/connections",
            connection(false),
        )
        .await;
        assert_eq!(
            created["data"]["session_ttl_minutes"], 4320,
            "default keeps 3 days"
        );
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_connection_needs_a_verified_domain_and_hides_its_secret() {
        let (state, path) = setup().await;
        let (status, _) = call(
            &state,
            "organization-a",
            Method::POST,
            "/connections",
            connection(true),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "no verified domain yet");

        let (status, body) = call(
            &state,
            "organization-a",
            Method::POST,
            "/connections",
            connection(false),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(body["data"]["has_client_secret"], true);
        assert!(!body.to_string().contains("s3cret"), "{body}");

        let (status, _) = call(
            &state,
            "organization-a",
            Method::POST,
            "/connections",
            json!({ "name": "x", "issuer": "http://idp.example", "client_id": "a" }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "plain http outside dev mode"
        );
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_domain_belongs_to_one_organization() {
        let (state, path) = setup().await;
        let (status, body) = call(
            &state,
            "organization-a",
            Method::POST,
            "/domains",
            json!({ "domain": "Contoso.com." }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(body["data"]["domain"], "contoso.com");
        assert_eq!(body["data"]["txt_name"], "_litepod-verify.contoso.com");
        let (status, _) = call(
            &state,
            "organization-b",
            Method::POST,
            "/domains",
            json!({ "domain": "contoso.com" }),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        let (status, _) = call(
            &state,
            "organization-b",
            Method::POST,
            "/domains",
            json!({ "domain": "not a domain" }),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn enforcement_needs_proof_and_then_refuses_passwords_except_the_sysadmin() {
        let (state, path) = setup().await;
        crate::ee::sso::provision::tests::seed_connection(&state).await;
        sqlx::query("UPDATE sso_domains SET domain = 'example.com'")
            .execute(&state.db)
            .await
            .unwrap();

        let (status, _) = call(
            &state,
            "organization-a",
            Method::PUT,
            "/enforcement",
            json!({ "enforced": true }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "editor never signed in with SSO"
        );
        sqlx::query("INSERT INTO user_identities (id, user_id, connection_id, subject) VALUES ('i', 'editor', 'c1', 'sub')")
            .execute(&state.db).await.unwrap();
        let (status, body) = call(
            &state,
            "organization-a",
            Method::PUT,
            "/enforcement",
            json!({ "enforced": true }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");

        sqlx::query("UPDATE users SET hashed_password = ?1 WHERE id = 'editor'")
            .bind(crate::utils::security::hash_password("pw".into()))
            .execute(&state.db)
            .await
            .unwrap();
        assert!(matches!(
            crate::cqrs::commands::auth::login(&state, "editor@example.com", "pw", None).await,
            Err(AppError::SsoRequired)
        ));
        assert!(
            matches!(
                crate::cqrs::commands::auth::login(&state, "editor@example.com", "wrong", None)
                    .await,
                Err(AppError::InvalidCredentials)
            ),
            "a wrong password still says nothing about SSO"
        );

        sqlx::query("INSERT INTO user_global_roles (user_id, role_id) VALUES ('editor', 0)")
            .execute(&state.db)
            .await
            .unwrap();
        assert!(
            crate::cqrs::commands::auth::login(&state, "editor@example.com", "pw", None)
                .await
                .is_ok(),
            "break-glass"
        );
        cleanup_database(&path);
    }

    async fn scim_status(is_cloud: bool) -> StatusCode {
        let (state, path) = test_state_for(is_cloud).await;
        let response = Router::new()
            .merge(crate::router::router::sso_routes_for_edition(state.clone()))
            .fallback(crate::router::router::ui_fallback)
            .with_state(state)
            .oneshot(
                Request::builder()
                    .uri("/scim/v2/organization-a/ServiceProviderConfig")
                    .header("authorization", "Bearer x")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        cleanup_database(&path);
        response.status()
    }

    async fn sso_start_status(is_cloud: bool) -> StatusCode {
        let (state, path) = test_state_for(is_cloud).await;
        let response = Router::new()
            .merge(crate::router::router::sso_routes_for_edition(state.clone()))
            .fallback(crate::router::router::ui_fallback)
            .with_state(state)
            .oneshot(
                Request::builder()
                    .uri("/api/v1/auth/sso/callback")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        cleanup_database(&path);
        response.status()
    }

    #[tokio::test]
    async fn on_cloud_none_of_it_exists() {
        assert_eq!(scim_status(true).await, StatusCode::NOT_FOUND);
        assert_eq!(
            scim_status(false).await,
            StatusCode::UNAUTHORIZED,
            "self-hosted: exists, token wrong"
        );
        assert_eq!(sso_start_status(true).await, StatusCode::NOT_FOUND);
        assert_eq!(
            sso_start_status(false).await,
            StatusCode::SEE_OTHER,
            "self-hosted: redirects to /login"
        );
    }
}
