//! Named reusable certificates. Private material is returned only to system administrators
//! explicitly editing a certificate; other certificate responses never include it.
use crate::{
    deployment::{
        certificates::{self, CertificateRequest, CertificateSettings},
        domains,
    },
    dtos::response::{AppError, ok},
    security::middleware::auth::AuthUser,
    state::state::SharedState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::header,
    response::IntoResponse,
};
use openssl::{
    pkey::PKey,
    ssl::{SslConnector, SslMethod, SslVerifyMode},
    stack::Stack,
    x509::{X509, X509PurposeId, X509StoreContext, store::X509StoreBuilder},
};
use serde::{Deserialize, Serialize};
use sqlx::Row;

#[derive(Serialize)]
pub struct CertificateDto {
    pub id: String,
    pub name: String,
    pub domains: Vec<String>,
    pub expires_at: String,
    pub in_use: bool,
}
#[derive(Serialize)]
pub struct CertificateMaterialDto {
    pub certificate_pem: String,
    pub private_key_pem: String,
}
#[derive(Deserialize)]
pub struct CertificateInput {
    pub name: Option<String>,
    pub certificate_pem: Option<String>,
    pub private_key_pem: Option<String>,
}

#[derive(Deserialize)]
pub struct CertificateTestInput {
    pub id: Option<String>,
    pub certificate_pem: Option<String>,
    pub private_key_pem: Option<String>,
}

#[derive(Serialize)]
pub struct CertificateDomainTest {
    pub host: String,
    pub covered: bool,
    pub served: Option<bool>,
    pub connection_error: Option<String>,
}

#[derive(Serialize)]
pub struct CertificateTestResult {
    pub key_matches: bool,
    pub dates_valid: bool,
    pub san_present: bool,
    pub chain_trusted: bool,
    pub chain_error: Option<String>,
    pub domains: Vec<CertificateDomainTest>,
}
fn db(error: sqlx::Error) -> AppError {
    AppError::DatabaseError(error.to_string())
}
fn invalid(message: &str) -> AppError {
    AppError::ValidationError(message.into())
}
fn require_admin(user: &AuthUser) -> Result<(), AppError> {
    if user.is_sysadmin {
        Ok(())
    } else {
        Err(AppError::InsufficientPermissions)
    }
}
fn name(value: &str) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 120 {
        return Err(invalid(
            "Certificate name must be between 1 and 120 characters",
        ));
    }
    Ok(value.into())
}
fn metadata(pem: &str) -> Result<(Vec<String>, String), AppError> {
    let certificate = openssl::x509::X509::from_pem(pem.as_bytes())
        .map_err(|_| invalid("Invalid PEM certificate"))?;
    let domains: Vec<String> = certificate
        .subject_alt_names()
        .map(|names| {
            names
                .iter()
                .filter_map(|name| name.dnsname().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    if domains.is_empty() {
        return Err(invalid(
            "Certificate must contain DNS subject alternative names",
        ));
    }
    let date = chrono::NaiveDateTime::parse_from_str(
        &certificate.not_after().to_string(),
        "%b %e %H:%M:%S %Y GMT",
    )
    .map_err(|_| invalid("Invalid certificate expiry date"))?;
    Ok((domains, date.and_utc().to_rfc3339()))
}
pub async fn load_settings(state: &SharedState, id: &str) -> Result<CertificateSettings, AppError> {
    let row = sqlx::query(
        "SELECT name, certificate_pem, encrypted_private_key FROM https_certificates WHERE id=?",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(db)?
    .ok_or(AppError::ResourceNotFound)?;
    Ok(CertificateSettings {
        certificate_id: Some(id.into()),
        certificate_name: Some(row.try_get(0).map_err(db)?),
        certificate_provider: "custom".into(),
        has_custom_certificate: true,
        certificate_pem: Some(row.try_get(1).map_err(db)?),
        encrypted_private_key: Some(row.try_get(2).map_err(db)?),
    })
}
pub async fn detail_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    require_admin(&user)?;
    let saved = load_settings(&state, &id).await?;
    let certificate_pem = saved
        .certificate_pem
        .ok_or_else(|| invalid("Missing certificate"))?;
    let private_key_pem = crate::utils::security::decrypt_data(
        saved
            .encrypted_private_key
            .as_deref()
            .ok_or_else(|| invalid("Missing private key"))?,
        &crate::utils::security::get_encryption_key(),
    )
    .map_err(|_| invalid("Could not access private key"))?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        ok(CertificateMaterialDto {
            certificate_pem,
            private_key_pem,
        }),
    ))
}
async fn dto(state: &SharedState, id: &str) -> Result<CertificateDto, AppError> {
    let saved = load_settings(state, id).await?;
    let (domains, expires_at) = metadata(saved.certificate_pem.as_deref().unwrap())?;
    let in_use: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM domain_certificates WHERE certificate_id=?)",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await
    .map_err(db)?;
    Ok(CertificateDto {
        id: id.into(),
        name: saved.certificate_name.unwrap_or_default(),
        domains,
        expires_at,
        in_use: in_use != 0,
    })
}
fn pair(
    input: &CertificateInput,
    previous: &CertificateSettings,
) -> Result<CertificateSettings, AppError> {
    if input.certificate_pem.is_none() && input.private_key_pem.is_none() {
        if previous.has_custom_certificate {
            return Ok(previous.clone());
        }
        return Err(invalid("Provide a PEM certificate chain and private key"));
    }
    let pem = input
        .certificate_pem
        .as_deref()
        .ok_or_else(|| invalid("Replace the certificate chain and private key together"))?;
    let (domains, _) = metadata(pem)?;
    // A wildcard itself is a valid SAN lookup here. Domain assignments validate the actual hostname separately.
    certificates::prepare(
        &Default::default(),
        &CertificateRequest {
            certificate_id: None,
            certificate_provider: Some("custom".into()),
            certificate_pem: input.certificate_pem.clone(),
            private_key_pem: input.private_key_pem.clone(),
        },
        &domains[0],
        true,
    )
}

async fn probe_host(host: String, expected_der: Vec<u8>) -> Result<bool, String> {
    let addresses = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::net::lookup_host((host.as_str(), 443)),
    )
    .await
    .map_err(|_| "DNS lookup timed out".to_string())?
    .map_err(|error| format!("DNS lookup failed: {error}"))?;
    let mut last_error = "No address found".to_string();
    for address in addresses.take(4) {
        let stream = match tokio::time::timeout(
            std::time::Duration::from_secs(3),
            tokio::net::TcpStream::connect(address),
        )
        .await
        {
            Ok(Ok(stream)) => stream,
            Ok(Err(error)) => {
                last_error = format!("Connection failed: {error}");
                continue;
            }
            Err(_) => {
                last_error = "Connection timed out".into();
                continue;
            }
        };
        let host = host.clone();
        let expected_der = expected_der.clone();
        return tokio::task::spawn_blocking(move || {
            let stream = stream.into_std().map_err(|error| error.to_string())?;
            stream
                .set_nonblocking(false)
                .map_err(|error| error.to_string())?;
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .map_err(|error| error.to_string())?;
            stream
                .set_write_timeout(Some(std::time::Duration::from_secs(3)))
                .map_err(|error| error.to_string())?;
            let mut builder =
                SslConnector::builder(SslMethod::tls()).map_err(|error| error.to_string())?;
            // Trust is reported separately. This handshake checks which leaf is actually served.
            builder.set_verify(SslVerifyMode::NONE);
            let connection = builder
                .build()
                .connect(&host, stream)
                .map_err(|error| error.to_string())?;
            let peer = connection
                .ssl()
                .peer_certificate()
                .ok_or("Server sent no certificate")?;
            Ok(peer.to_der().map_err(|error| error.to_string())? == expected_der)
        })
        .await
        .map_err(|error| error.to_string())?;
    }
    Err(last_error)
}

fn verify_chain_trust(chain: &[X509]) -> (bool, Option<String>) {
    match (|| -> Result<(bool, Option<String>), openssl::error::ErrorStack> {
        let mut store = X509StoreBuilder::new()?;
        store.set_default_paths()?;
        store.set_purpose(X509PurposeId::SSL_SERVER)?;
        let store = store.build();
        let mut intermediates = Stack::new()?;
        for certificate in chain.iter().skip(1) {
            intermediates.push(certificate.to_owned())?;
        }
        let mut context = X509StoreContext::new()?;
        let trusted = context.init(&store, &chain[0], &intermediates, |context| {
            context.verify_cert()
        })?;
        Ok((
            trusted,
            (!trusted).then(|| context.error().error_string().to_string()),
        ))
    })() {
        Ok(result) => result,
        Err(error) => (false, Some(error.to_string())),
    }
}

pub async fn test_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    Json(input): Json<CertificateTestInput>,
) -> Result<impl IntoResponse, AppError> {
    require_admin(&user)?;
    let saved = match input.id.as_deref() {
        Some(id) => Some(load_settings(&state, id).await?),
        None => None,
    };
    let (pem, key) = match (
        input.certificate_pem.as_deref(),
        input.private_key_pem.as_deref(),
    ) {
        (Some(pem), Some(key)) => (pem.to_owned(), key.to_owned()),
        (None, None) => {
            let saved = saved
                .as_ref()
                .ok_or_else(|| invalid("Provide a PEM certificate chain and private key"))?;
            (
                saved
                    .certificate_pem
                    .clone()
                    .ok_or_else(|| invalid("Missing certificate"))?,
                crate::utils::security::decrypt_data(
                    saved
                        .encrypted_private_key
                        .as_deref()
                        .ok_or_else(|| invalid("Missing private key"))?,
                    &crate::utils::security::get_encryption_key(),
                )
                .map_err(|_| invalid("Could not access private key"))?,
            )
        }
        _ => {
            return Err(invalid(
                "Provide the certificate chain and private key together",
            ));
        }
    };
    if pem.len() > 262144 || key.len() > 32768 {
        return Err(invalid("Certificate or private key exceeds the size limit"));
    }
    let chain = X509::stack_from_pem(pem.as_bytes())
        .map_err(|_| invalid("Invalid PEM certificate chain"))?;
    let leaf = chain
        .first()
        .ok_or_else(|| invalid("Certificate chain is empty"))?;
    let key = PKey::private_key_from_pem(key.as_bytes())
        .map_err(|_| invalid("Invalid PEM private key"))?;
    let key_matches = leaf
        .public_key()
        .map_err(|_| invalid("Invalid certificate public key"))?
        .public_eq(&key);
    let now = openssl::asn1::Asn1Time::days_from_now(0)
        .map_err(|_| invalid("Could not check certificate validity"))?;
    let dates_valid = chain
        .iter()
        .all(|cert| cert.not_before() <= now && cert.not_after() > now);
    let san_present = leaf
        .subject_alt_names()
        .is_some_and(|names| names.iter().any(|name| name.dnsname().is_some()));
    let (chain_trusted, chain_error) = verify_chain_trust(&chain);
    let mut domains = Vec::new();
    if let Some(id) = input.id.as_deref() {
        for route in assigned_routes(&state, id).await? {
            let host = if route == "litepod-server-domain" {
                crate::handler::settings::load(&state)
                    .await?
                    .domain
                    .ok_or_else(|| invalid("Missing panel domain"))?
            } else {
                domains::get_by_id(
                    &state,
                    route
                        .strip_prefix("litepod-route-")
                        .ok_or_else(|| invalid("Invalid certificate assignment"))?,
                )
                .await?
                .host
            };
            let covered = leaf.subject_alt_names().is_some_and(|names| {
                names
                    .iter()
                    .filter_map(|name| name.dnsname())
                    .any(|name| certificates::hostname_matches(name, &host))
            });
            let (served, connection_error) = match probe_host(
                host.clone(),
                leaf.to_der().map_err(|_| invalid("Invalid certificate"))?,
            )
            .await
            {
                Ok(served) => (Some(served), None),
                Err(error) => (None, Some(error)),
            };
            domains.push(CertificateDomainTest {
                host,
                covered,
                served,
                connection_error,
            });
        }
    }
    Ok(ok(CertificateTestResult {
        key_matches,
        dates_valid,
        san_present,
        chain_trusted,
        chain_error,
        domains,
    }))
}
pub async fn list_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    require_admin(&user)?;
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM https_certificates ORDER BY name, id")
            .fetch_all(&state.db)
            .await
            .map_err(db)?;
    let mut result = Vec::new();
    for id in ids {
        result.push(dto(&state, &id).await?);
    }
    Ok(ok(result))
}
pub async fn create_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    Json(input): Json<CertificateInput>,
) -> Result<impl IntoResponse, AppError> {
    require_admin(&user)?;
    let _guard = certificates::INVENTORY_LOCK.lock().await;
    let label = name(input.name.as_deref().unwrap_or_default())?;
    let saved = pair(&input, &Default::default())?;
    let id = uuid::Uuid::now_v7().to_string();
    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query("INSERT INTO https_certificates (id,name,certificate_pem,encrypted_private_key,created_at,updated_at) VALUES (?,?,?,?,?,?)").bind(&id).bind(&label).bind(&saved.certificate_pem).bind(&saved.encrypted_private_key).bind(now).bind(now).execute(&state.db).await.map_err(db)?;
    let _ = crate::security::security_events::record(
        &state,
        None,
        &user,
        "certificate.created",
        Some(&label),
    )
    .await;
    Ok(ok(dto(&state, &id).await?))
}
async fn assigned_routes(state: &SharedState, id: &str) -> Result<Vec<String>, AppError> {
    sqlx::query_scalar(
        "SELECT route_id FROM domain_certificates WHERE certificate_id=? ORDER BY route_id",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .map_err(db)
}
async fn sync_routes(state: &SharedState, routes: &[String]) -> Result<(), AppError> {
    for route in routes {
        if route == "litepod-server-domain" {
            crate::handler::settings::restore_caddy(state).await?;
        } else if let Some(id) = route.strip_prefix("litepod-route-") {
            let domain = domains::get_by_id(state, id).await?;
            if domains::application_uses_local_caddy(state, &domain.application_id).await? {
                if let Some((container, port)) =
                    domains::caddy_target(state, &domain.application_id, &domain).await?
                {
                    domains::push_caddy_route(state, &domain, &container, port).await?;
                }
            } else {
                crate::servers::ingress::sync_domain(state, &domain).await?;
            }
        }
    }
    Ok(())
}
async fn persist(
    state: &SharedState,
    id: &str,
    label: &str,
    saved: &CertificateSettings,
) -> Result<(), AppError> {
    sqlx::query("UPDATE https_certificates SET name=?,certificate_pem=?,encrypted_private_key=?,updated_at=? WHERE id=?").bind(label).bind(&saved.certificate_pem).bind(&saved.encrypted_private_key).bind(chrono::Utc::now().timestamp_millis()).bind(id).execute(&state.db).await.map_err(db)?;
    Ok(())
}
pub async fn update_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(input): Json<CertificateInput>,
) -> Result<impl IntoResponse, AppError> {
    require_admin(&user)?;
    let _guard = certificates::INVENTORY_LOCK.lock().await;
    let previous = load_settings(&state, &id).await?;
    let label = name(
        input
            .name
            .as_deref()
            .unwrap_or(previous.certificate_name.as_deref().unwrap_or_default()),
    )?;
    let saved = pair(&input, &previous)?;
    let routes = assigned_routes(&state, &id).await?;
    for route in &routes {
        let host = if route == "litepod-server-domain" {
            crate::handler::settings::load(&state)
                .await?
                .domain
                .ok_or_else(|| invalid("Missing panel domain"))?
        } else {
            domains::get_by_id(
                &state,
                route
                    .strip_prefix("litepod-route-")
                    .ok_or_else(|| invalid("Invalid certificate assignment"))?,
            )
            .await?
            .host
        };
        certificates::prepare(&saved, &Default::default(), &host, true)?;
    }
    persist(&state, &id, &label, &saved).await?;
    if input.certificate_pem.is_some() {
        if let Err(error) = sync_routes(&state, &routes).await {
            persist(
                &state,
                &id,
                previous.certificate_name.as_deref().unwrap_or_default(),
                &previous,
            )
            .await?;
            if sync_routes(&state, &routes).await.is_err() {
                tracing::warn!(certificate_id=%id, "Certificate rotation rollback requires route reconciliation");
            }
            return Err(error);
        }
    }
    let _ = crate::security::security_events::record(
        &state,
        None,
        &user,
        "certificate.updated",
        Some(&label),
    )
    .await;
    Ok(ok(dto(&state, &id).await?))
}
pub async fn delete_handler(
    State(state): State<SharedState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    require_admin(&user)?;
    let _guard = certificates::INVENTORY_LOCK.lock().await;
    let saved = load_settings(&state, &id).await?;
    if !assigned_routes(&state, &id).await?.is_empty() {
        return Err(AppError::ResourceInUse(
            "Certificate is assigned to a domain; change its provider or certificate first".into(),
        ));
    }
    sqlx::query("DELETE FROM https_certificates WHERE id=?")
        .bind(&id)
        .execute(&state.db)
        .await
        .map_err(db)?;
    let _ = crate::security::security_events::record(
        &state,
        None,
        &user,
        "certificate.deleted",
        Some(saved.certificate_name.as_deref().unwrap_or(id.as_str())),
    )
    .await;
    Ok(ok(serde_json::json!({"deleted":true})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_signed_certificate_is_valid_but_not_trusted_by_default() {
        let (pem, key) = certificates::tests::pair("test.example.com", false);
        assert!(certificates::validate_pair("test.example.com", &pem, &key).is_ok());
        let chain = X509::stack_from_pem(pem.as_bytes()).unwrap();
        let (trusted, reason) = verify_chain_trust(&chain);
        assert!(!trusted);
        assert!(reason.is_some());
    }

    #[tokio::test]
    async fn inventory_hides_keys_validates_assignments_and_refuses_used_deletion() {
        let (state, path) = crate::testing::test_state().await;
        let user = crate::testing::sysadmin_user("editor");
        let (pem, key) = certificates::tests::pair("*.example.com", false);
        assert!(
            list_handler(
                State(state.clone()),
                Extension(crate::testing::auth_user("editor"))
            )
            .await
            .is_err()
        );
        let response = create_handler(
            State(state.clone()),
            Extension(user.clone()),
            Json(CertificateInput {
                name: Some("Wildcard production".into()),
                certificate_pem: Some(pem),
                private_key_pem: Some(key),
            }),
        )
        .await
        .unwrap()
        .into_response();
        let body = axum::body::to_bytes(response.into_body(), 16384)
            .await
            .unwrap();
        let text = std::str::from_utf8(&body).unwrap();
        assert!(!text.contains("BEGIN"));
        assert!(!text.contains("encrypted_private_key"));
        let id: String = sqlx::query_scalar("SELECT id FROM https_certificates")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert!(
            detail_handler(
                State(state.clone()),
                Extension(crate::testing::auth_user("editor")),
                Path(id.clone())
            )
            .await
            .is_err()
        );
        let detail = detail_handler(
            State(state.clone()),
            Extension(user.clone()),
            Path(id.clone()),
        )
        .await
        .unwrap()
        .into_response();
        assert_eq!(
            detail.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store"
        );
        let detail_body = axum::body::to_bytes(detail.into_body(), 16384)
            .await
            .unwrap();
        let detail_text = std::str::from_utf8(&detail_body).unwrap();
        assert!(detail_text.contains("BEGIN CERTIFICATE"));
        assert!(detail_text.contains("BEGIN PRIVATE KEY"));
        let request = CertificateRequest {
            certificate_provider: Some("custom".into()),
            certificate_id: Some(id.clone()),
            ..Default::default()
        };
        let assigned = certificates::prepare_for_state(
            &state,
            &Default::default(),
            &request,
            "api.example.com",
            true,
        )
        .await
        .unwrap();
        assert_eq!(assigned.certificate_id.as_deref(), Some(id.as_str()));
        assert!(
            certificates::prepare_for_state(
                &state,
                &Default::default(),
                &request,
                "wrong.invalid",
                true
            )
            .await
            .is_err()
        );
        let mut conn = state.db.acquire().await.unwrap();
        certificates::save_on(&mut conn, "litepod-server-domain", &assigned)
            .await
            .unwrap();
        drop(conn);
        let stored: Option<String> = sqlx::query_scalar("SELECT encrypted_private_key FROM domain_certificates WHERE route_id='litepod-server-domain'").fetch_one(&state.db).await.unwrap();
        assert!(
            stored.is_none(),
            "assignments reference the encrypted inventory entry, never duplicate private keys"
        );
        let loaded = certificates::load(&state, "litepod-server-domain")
            .await
            .unwrap();
        assert_eq!(
            loaded.certificate_name.as_deref(),
            Some("Wildcard production")
        );
        sqlx::query("INSERT INTO server_domain_settings (id,domain,email,https,updated_at) VALUES (1,'panel.example.com','admin@example.com',1,0)").execute(&state.db).await.unwrap();
        let before = load_settings(&state, &id).await.unwrap();
        let (wrong_pem, wrong_key) = certificates::tests::pair("other.invalid", false);
        assert!(
            update_handler(
                State(state.clone()),
                Extension(user.clone()),
                Path(id.clone()),
                Json(CertificateInput {
                    name: None,
                    certificate_pem: Some(wrong_pem),
                    private_key_pem: Some(wrong_key)
                })
            )
            .await
            .is_err()
        );
        assert_eq!(
            load_settings(&state, &id).await.unwrap().certificate_pem,
            before.certificate_pem
        );
        let (replacement, replacement_key) = certificates::tests::pair("*.example.com", false);
        // The test Caddy URL is unavailable: failed renewal must restore the entire inventory entry.
        assert!(
            update_handler(
                State(state.clone()),
                Extension(user.clone()),
                Path(id.clone()),
                Json(CertificateInput {
                    name: Some("Should roll back".into()),
                    certificate_pem: Some(replacement),
                    private_key_pem: Some(replacement_key)
                })
            )
            .await
            .is_err()
        );
        let after = load_settings(&state, &id).await.unwrap();
        assert_eq!(after.certificate_pem, before.certificate_pem);
        assert_eq!(after.encrypted_private_key, before.encrypted_private_key);
        assert_eq!(after.certificate_name, before.certificate_name);
        assert!(
            delete_handler(
                State(state.clone()),
                Extension(user.clone()),
                Path(id.clone())
            )
            .await
            .is_err()
        );
        sqlx::query("DELETE FROM domain_certificates WHERE route_id='litepod-server-domain'")
            .execute(&state.db)
            .await
            .unwrap();
        update_handler(
            State(state.clone()),
            Extension(user.clone()),
            Path(id.clone()),
            Json(CertificateInput {
                name: Some("Renamed".into()),
                certificate_pem: None,
                private_key_pem: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(dto(&state, &id).await.unwrap().name, "Renamed");
        delete_handler(State(state.clone()), Extension(user), Path(id))
            .await
            .unwrap();
        crate::testing::cleanup_database(&path);
    }
}
