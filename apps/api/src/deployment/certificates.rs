//! Certificate settings shared by application and control-panel domains.
use crate::{
    dtos::response::AppError,
    state::state::SharedState,
    utils::security::{decrypt_data, encrypt_data, get_encryption_key},
};
use openssl::{asn1::Asn1Time, pkey::PKey, x509::X509};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::Row;

#[derive(Clone, Default, Deserialize, utoipa::ToSchema)]
pub struct CertificateRequest {
    pub certificate_id: Option<String>,
    pub certificate_provider: Option<String>,
    pub certificate_pem: Option<String>,
    pub private_key_pem: Option<String>,
}
impl std::fmt::Debug for CertificateRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CertificateRequest")
            .field("provider", &self.certificate_provider)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Serialize)]
pub struct CertificateSettings {
    pub certificate_id: Option<String>,
    pub certificate_name: Option<String>,
    pub certificate_provider: String,
    pub has_custom_certificate: bool,
    #[serde(skip)]
    pub certificate_pem: Option<String>,
    #[serde(skip)]
    pub encrypted_private_key: Option<String>,
}
impl std::fmt::Debug for CertificateSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CertificateSettings")
            .field("provider", &self.certificate_provider)
            .finish_non_exhaustive()
    }
}
impl Default for CertificateSettings {
    fn default() -> Self {
        Self {
            certificate_id: None,
            certificate_name: None,
            certificate_provider: "automatic".into(),
            has_custom_certificate: false,
            certificate_pem: None,
            encrypted_private_key: None,
        }
    }
}
fn invalid(message: &str) -> AppError {
    AppError::ValidationError(message.into())
}
fn db(error: sqlx::Error) -> AppError {
    AppError::DatabaseError(error.to_string())
}
fn secret_error(_: String) -> AppError {
    AppError::InternalError("Could not access certificate private key".into())
}

pub fn validate_pair(host: &str, pem: &str, key: &str) -> Result<(), AppError> {
    if pem.len() > 262144 || key.len() > 32768 {
        return Err(invalid("Certificate or private key exceeds the size limit"));
    }
    let chain = X509::stack_from_pem(pem.as_bytes())
        .map_err(|_| invalid("Invalid PEM certificate chain"))?;
    let cert = chain
        .first()
        .ok_or_else(|| invalid("Certificate chain is empty"))?;
    let key = PKey::private_key_from_pem(key.as_bytes())
        .map_err(|_| invalid("Provide an unencrypted PEM private key"))?;
    if !cert
        .public_key()
        .map_err(|_| invalid("Invalid certificate public key"))?
        .public_eq(&key)
    {
        return Err(invalid("Certificate and private key do not match"));
    }
    let now =
        Asn1Time::days_from_now(0).map_err(|_| invalid("Could not check certificate validity"))?;
    for cert in &chain {
        if cert.not_before() > now || cert.not_after() <= now {
            return Err(invalid(
                "Certificate chain contains an expired or not-yet-valid certificate",
            ));
        }
    }
    let matches = cert.subject_alt_names().is_some_and(|names| {
        names
            .iter()
            .filter_map(|name| name.dnsname())
            .any(|name| hostname_matches(name, host))
    });
    if !matches {
        return Err(invalid(
            "Certificate subject alternative names do not cover this domain",
        ));
    }
    Ok(())
}
pub(crate) fn hostname_matches(pattern: &str, host: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    let host = host.to_ascii_lowercase();
    pattern == host
        || pattern.strip_prefix("*.").is_some_and(|suffix| {
            host.split_once('.')
                .is_some_and(|(label, rest)| !label.is_empty() && rest == suffix)
        })
}
pub async fn load(state: &SharedState, id: &str) -> Result<CertificateSettings, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let Some(row) = sqlx::query("SELECT d.provider, COALESCE(c.certificate_pem, d.certificate_pem), COALESCE(c.encrypted_private_key, d.encrypted_private_key), d.certificate_id, c.name FROM domain_certificates d LEFT JOIN https_certificates c ON c.id=d.certificate_id WHERE d.route_id = ?").bind(id).fetch_optional(&conn).await.map_err(db)? else { return Ok(CertificateSettings::default()); };
    let certificate_pem: Option<String> = row.try_get(1).map_err(db)?;
    Ok(CertificateSettings {
        certificate_id: row.try_get(3).map_err(db)?,
        certificate_name: row.try_get(4).map_err(db)?,
        certificate_provider: row.try_get(0).map_err(db)?,
        has_custom_certificate: certificate_pem.is_some(),
        certificate_pem,
        encrypted_private_key: row.try_get(2).map_err(db)?,
    })
}
/// Shared with inventory rotation/deletion so references cannot change mid-update.
pub static INVENTORY_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn prepare_for_state(
    state: &SharedState,
    current: &CertificateSettings,
    request: &CertificateRequest,
    host: &str,
    https: bool,
) -> Result<CertificateSettings, AppError> {
    let Some(id) = request.certificate_id.as_deref() else {
        return prepare(current, request, host, https);
    };
    if request.certificate_pem.is_some() || request.private_key_pem.is_some() {
        return Err(invalid(
            "Choose a saved certificate or upload a PEM pair, not both",
        ));
    }
    if request
        .certificate_provider
        .as_deref()
        .unwrap_or(&current.certificate_provider)
        != "custom"
    {
        return Err(invalid("Saved certificates require the custom provider"));
    }
    let source = crate::handler::certificates::load_settings(state, id).await?;
    prepare(&source, request, host, https)
}

pub fn prepare(
    current: &CertificateSettings,
    request: &CertificateRequest,
    host: &str,
    https: bool,
) -> Result<CertificateSettings, AppError> {
    let provider = request
        .certificate_provider
        .as_deref()
        .unwrap_or(&current.certificate_provider);
    if !matches!(provider, "automatic" | "letsencrypt" | "custom") {
        return Err(invalid("Unknown HTTPS certificate provider"));
    }
    if provider != "custom" {
        if request.certificate_pem.is_some() || request.private_key_pem.is_some() {
            return Err(invalid("PEM certificates require the custom provider"));
        }
        return Ok(CertificateSettings {
            certificate_provider: provider.into(),
            ..Default::default()
        });
    }
    if !https {
        return Err(invalid("Custom certificates require HTTPS"));
    }
    let (cert, encrypted) = match (&request.certificate_pem, &request.private_key_pem) {
        (Some(cert), Some(key)) => {
            validate_pair(host, cert, key)?;
            (
                String::from_utf8(
                    X509::stack_from_pem(cert.as_bytes())
                        .map_err(|_| invalid("Invalid PEM certificate chain"))?
                        .iter()
                        .map(|cert| cert.to_pem())
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|_| invalid("Invalid PEM certificate chain"))?
                        .concat(),
                )
                .map_err(|_| invalid("Invalid PEM certificate chain"))?,
                encrypt_data(key, &get_encryption_key()).map_err(secret_error)?,
            )
        }
        (None, None) => {
            let cert = current
                .certificate_pem
                .clone()
                .ok_or_else(|| invalid("Provide a PEM certificate chain and private key"))?;
            let encrypted = current
                .encrypted_private_key
                .clone()
                .ok_or_else(|| invalid("Provide a PEM private key"))?;
            validate_pair(
                host,
                &cert,
                &decrypt_data(&encrypted, &get_encryption_key()).map_err(secret_error)?,
            )?;
            (cert, encrypted)
        }
        _ => {
            return Err(invalid(
                "Replace the certificate chain and private key together",
            ));
        }
    };
    Ok(CertificateSettings {
        certificate_id: if request.certificate_pem.is_none() {
            current.certificate_id.clone()
        } else {
            None
        },
        certificate_name: if request.certificate_pem.is_none() {
            current.certificate_name.clone()
        } else {
            None
        },
        certificate_provider: provider.into(),
        has_custom_certificate: true,
        certificate_pem: Some(cert),
        encrypted_private_key: Some(encrypted),
    })
}
pub async fn save_on(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
    settings: &CertificateSettings,
) -> Result<(), AppError> {
    sqlx::query("INSERT INTO domain_certificates (route_id, provider, certificate_pem, encrypted_private_key, certificate_id) VALUES (?, ?, ?, ?, ?) ON CONFLICT(route_id) DO UPDATE SET provider=excluded.provider, certificate_pem=excluded.certificate_pem, encrypted_private_key=excluded.encrypted_private_key, certificate_id=excluded.certificate_id").bind(id).bind(&settings.certificate_provider).bind(if settings.certificate_id.is_some() { None } else { settings.certificate_pem.as_deref() }).bind(if settings.certificate_id.is_some() { None } else { settings.encrypted_private_key.as_deref() }).bind(&settings.certificate_id).execute(conn).await.map_err(db)?;
    Ok(())
}
/// Upload a private certificate through SSH, never through shell arguments.
pub async fn install_remote(
    state: &SharedState,
    server: &crate::servers::repository::ServerRecord,
    application_id: &str,
    settings: &CertificateSettings,
) -> Result<String, AppError> {
    let cert = settings
        .certificate_pem
        .as_deref()
        .ok_or_else(|| invalid("Missing custom certificate"))?;
    let key = decrypt_data(
        settings
            .encrypted_private_key
            .as_deref()
            .ok_or_else(|| invalid("Missing private key"))?,
        &get_encryption_key(),
    )
    .map_err(secret_error)?;
    let token = crate::utils::security::digest_token(cert);
    let file = crate::servers::ssh::SecretFile::write(&format!("{cert}\n{key}"))?;
    let runtime =
        crate::servers::runtime::ServerRuntime::for_application(state, application_id).await?;
    let path = runtime
        .upload_workspace_file(
            application_id,
            file.path(),
            &format!("caddy-certificate-{token}.pem"),
        )
        .await?
        .ok_or_else(|| invalid("Expected a remote server"))?;
    crate::servers::setup::caddy_helper(
        server,
        &[
            "install-certificate".into(),
            path.into(),
            token.clone().into(),
        ],
    )
    .await
    .map_err(|_| {
        AppError::InternalError(
            "Could not install remote certificate; repair the server and retry".into(),
        )
    })?;
    Ok(token)
}

/// Detect provider/PEM drift as well as missing routes after a failed rotation rollback.
pub fn matches_config(
    config: &Value,
    id: &str,
    https: bool,
    settings: &CertificateSettings,
) -> bool {
    let tag = format!("{id}-certificate");
    let loaded = config
        .pointer("/apps/tls/certificates/load_pem")
        .and_then(Value::as_array)
        .and_then(|entries| {
            entries.iter().find(|entry| {
                entry["tags"]
                    .as_array()
                    .is_some_and(|tags| tags.iter().any(|v| v.as_str() == Some(&tag)))
            })
        });
    if !https {
        return loaded.is_none();
    }
    if settings.certificate_provider == "custom" {
        return loaded.is_some_and(|entry| {
            entry["certificate"].as_str() == settings.certificate_pem.as_deref()
        });
    }
    if loaded.is_some() {
        return false;
    }
    let policy_id = format!("{id}-tls");
    let policy = config
        .pointer("/apps/tls/automation/policies")
        .and_then(Value::as_array)
        .and_then(|policies| {
            policies
                .iter()
                .find(|policy| policy["@id"].as_str() == Some(&policy_id))
        });
    match settings.certificate_provider.as_str() {
        "letsencrypt" => policy.is_some_and(|policy| {
            policy.pointer("/issuers/0/ca").and_then(Value::as_str)
                == Some("https://acme-v02.api.letsencrypt.org/directory")
        }),
        _ => policy.is_some_and(|policy| policy.get("issuers").is_none()),
    }
}

pub fn apply(
    config: &mut Value,
    id: &str,
    host: &str,
    https: bool,
    settings: &CertificateSettings,
    email: Option<&str>,
) -> Result<(), AppError> {
    let tag = format!("{id}-certificate");
    let tls = config["apps"]
        .as_object_mut()
        .ok_or_else(|| invalid("Invalid Caddy apps"))?
        .entry("tls")
        .or_insert_with(|| json!({}));
    let tls = tls
        .as_object_mut()
        .ok_or_else(|| invalid("Invalid Caddy TLS configuration"))?;
    let certificates = tls
        .entry("certificates")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| invalid("Invalid Caddy certificates"))?;
    let loaded = certificates
        .entry("load_pem")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| invalid("Invalid Caddy PEM loader"))?;
    loaded.retain(|entry| {
        !entry["tags"]
            .as_array()
            .is_some_and(|tags| tags.iter().any(|v| v.as_str() == Some(&tag)))
    });
    if https && settings.certificate_provider == "custom" {
        let key = decrypt_data(
            settings
                .encrypted_private_key
                .as_deref()
                .ok_or_else(|| invalid("Missing certificate private key"))?,
            &get_encryption_key(),
        )
        .map_err(secret_error)?;
        loaded.push(json!({"certificate": settings.certificate_pem, "key": key, "tags": [tag]}));
    }
    if let Some(policies) = tls
        .get_mut("automation")
        .and_then(|v| v.get_mut("policies"))
        .and_then(Value::as_array_mut)
    {
        let policy_id = format!("{id}-tls");
        if let Some(policy) = policies
            .iter_mut()
            .find(|p| p["@id"].as_str() == Some(&policy_id))
        {
            if https && settings.certificate_provider == "letsencrypt" {
                policy["issuers"] = json!([{"module":"acme", "ca":"https://acme-v02.api.letsencrypt.org/directory", "email":email.unwrap_or("")}]);
            }
        }
        if settings.certificate_provider == "custom" {
            policies.retain(|p| p["@id"].as_str() != Some(&policy_id));
        }
    }
    let server = &mut config["apps"]["http"]["servers"]["srv0"];
    let custom = https && settings.certificate_provider == "custom";
    if let Some(policies) = server["tls_connection_policies"].as_array_mut() {
        for policy in policies {
            if policy
                .pointer("/match/sni")
                .and_then(Value::as_array)
                .is_some_and(|names| names.iter().any(|name| name == host))
            {
                if custom {
                    policy["certificate_selection"] = json!({"any_tag": [tag]});
                } else if let Some(policy) = policy.as_object_mut() {
                    policy.remove("certificate_selection");
                }
            }
        }
    }
    if custom {
        if server.get("automatic_https").is_none() {
            server["automatic_https"] = json!({});
        }
        if server["automatic_https"].get("skip_certificates").is_none() {
            server["automatic_https"]["skip_certificates"] = json!([]);
        }
    }
    if let Some(auto) = server
        .get_mut("automatic_https")
        .and_then(Value::as_object_mut)
    {
        if let Some(skip) = auto
            .get_mut("skip_certificates")
            .and_then(Value::as_array_mut)
        {
            skip.retain(|v| v.as_str() != Some(host));
            if custom {
                skip.push(json!(host));
            }
            if skip.is_empty() {
                auto.remove("skip_certificates");
            }
        }
        if auto.is_empty() {
            server.as_object_mut().unwrap().remove("automatic_https");
        }
    }
    Ok(())
}
pub fn remove_from_config(config: &mut Value, id: &str, host: &str) {
    let tag = format!("{id}-certificate");
    if let Some(loaded) = config
        .pointer_mut("/apps/tls/certificates/load_pem")
        .and_then(Value::as_array_mut)
    {
        loaded.retain(|entry| {
            !entry["tags"]
                .as_array()
                .is_some_and(|tags| tags.iter().any(|v| v.as_str() == Some(&tag)))
        });
    }
    if let Some(skip) = config
        .pointer_mut("/apps/http/servers/srv0/automatic_https/skip_certificates")
        .and_then(Value::as_array_mut)
    {
        skip.retain(|v| v.as_str() != Some(host));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use openssl::{
        hash::MessageDigest,
        rsa::Rsa,
        x509::{X509NameBuilder, extension::SubjectAlternativeName},
    };

    pub fn pair(host: &str, expired: bool) -> (String, String) {
        let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
        let mut name = X509NameBuilder::new().unwrap();
        name.append_entry_by_text("CN", host).unwrap();
        let name = name.build();
        let mut cert = X509::builder().unwrap();
        cert.set_version(2).unwrap();
        cert.set_subject_name(&name).unwrap();
        cert.set_issuer_name(&name).unwrap();
        cert.set_pubkey(&key).unwrap();
        cert.set_not_before(&Asn1Time::from_unix(1).unwrap())
            .unwrap();
        let until = if expired {
            Asn1Time::from_unix(2).unwrap()
        } else {
            Asn1Time::days_from_now(30).unwrap()
        };
        cert.set_not_after(&until).unwrap();
        let san = SubjectAlternativeName::new()
            .dns(host)
            .build(&cert.x509v3_context(None, None))
            .unwrap();
        cert.append_extension(san).unwrap();
        cert.sign(&key, MessageDigest::sha256()).unwrap();
        (
            String::from_utf8(cert.build().to_pem().unwrap()).unwrap(),
            String::from_utf8(key.private_key_to_pem_pkcs8().unwrap()).unwrap(),
        )
    }

    #[test]
    fn certificates_validate_identity_dates_and_preserve_private_material() {
        let (cert, key) = pair("*.example.com", false);
        assert!(validate_pair("api.example.com", &cert, &key).is_ok());
        assert!(validate_pair("a.api.example.com", &cert, &key).is_err());
        assert!(validate_pair("example.com", &cert, &key).is_err());
        assert!(validate_pair("evil-example.com", &cert, &key).is_err());
        let (_, wrong_key) = pair("api.example.com", false);
        assert!(validate_pair("api.example.com", &cert, &wrong_key).is_err());
        let (expired, expired_key) = pair("api.example.com", true);
        assert!(validate_pair("api.example.com", &expired, &expired_key).is_err());
        let settings = prepare(
            &Default::default(),
            &CertificateRequest {
                certificate_provider: Some("custom".into()),
                certificate_pem: Some(cert.clone()),
                private_key_pem: Some(key.clone()),
                certificate_id: None,
            },
            "api.example.com",
            true,
        )
        .unwrap();
        assert!(
            !settings
                .encrypted_private_key
                .as_ref()
                .unwrap()
                .contains("PRIVATE KEY")
        );
        let response = serde_json::to_string(&settings).unwrap();
        assert!(!response.contains("BEGIN"));
        assert!(!response.contains("encrypted_private_key"));
        let retained = prepare(&settings, &Default::default(), "api.example.com", true).unwrap();
        assert_eq!(
            retained.encrypted_private_key,
            settings.encrypted_private_key
        );
        assert!(prepare(&settings, &Default::default(), "other.invalid", true).is_err());
        let mut config = json!({"apps":{"http":{"servers":{"srv0":{}}},"tls":{"automation":{"policies":[{"@id":"route-tls","subjects":["api.example.com"]}]}}}});
        apply(
            &mut config,
            "route",
            "api.example.com",
            true,
            &settings,
            None,
        )
        .unwrap();
        assert_eq!(
            config["apps"]["tls"]["certificates"]["load_pem"][0]["key"],
            key
        );
        assert!(
            config["apps"]["tls"]["automation"]["policies"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let mut debug = config.clone();
        crate::handler::debug::redact_value(&mut debug);
        assert!(!debug.to_string().contains("PRIVATE KEY"));
        remove_from_config(&mut config, "route", "api.example.com");
        assert!(
            config["apps"]["tls"]["certificates"]["load_pem"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            config["apps"]["http"]["servers"]["srv0"]["automatic_https"]["skip_certificates"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let le = CertificateSettings {
            certificate_provider: "letsencrypt".into(),
            ..Default::default()
        };
        config["apps"]["tls"]["automation"]["policies"] =
            json!([{"@id":"route-tls","subjects":["api.example.com"]}]);
        apply(
            &mut config,
            "route",
            "api.example.com",
            true,
            &le,
            Some("admin@example.com"),
        )
        .unwrap();
        assert_eq!(
            config["apps"]["tls"]["automation"]["policies"][0]["issuers"][0]["ca"],
            "https://acme-v02.api.letsencrypt.org/directory"
        );
    }
}
