//! SAML 2.0, SP-initiated (spec §7). Behind the `saml` cargo feature because
//! `samael` needs native libxml2 + xmlsec1. Same `provision()` path as OIDC.
//!
//! Trust comes from three checks `samael` does for us — the IdP signature
//! (only when the IdP metadata carries a signing certificate, so we refuse
//! metadata without one), audience/recipient/destination, and `InResponseTo`
//! against the single-use request id we stored.

use samael::crypto::AllowedSignatureAlgorithm;
use samael::metadata::{EntityDescriptor, HTTP_REDIRECT_BINDING};
use samael::schema::Assertion;
use samael::service_provider::{ServiceProvider, ServiceProviderBuilder};
use samael::traits::ToXml;

use super::provision::{Identity, provision};
use super::{
    Connection, Finish, Gate, REAUTH_REFUSED, SsoSession, connection, db, gate, save_login_state,
    take_login_state,
};
use crate::dtos::response::AppError;
use crate::state::state::SharedState;
use crate::utils::security;

const EMAIL_ATTRIBUTES: &[&str] = &[
    "http://schemas.xmlsoap.org/ws/2005/05/identity/claims/emailaddress",
    "email",
    "mail",
    "urn:oid:0.9.2342.19200300.100.1.3",
];
const NAME_ATTRIBUTES: &[&str] = &[
    "http://schemas.microsoft.com/identity/claims/displayname",
    "http://schemas.xmlsoap.org/ws/2005/05/identity/claims/name",
    "displayName",
    "name",
];

fn saml_error(context: &str, e: impl std::fmt::Display) -> AppError {
    tracing::warn!(error = %e, "saml: {context}");
    AppError::ValidationError(format!("Single sign-on failed: {context}"))
}

/// The SP entity id, which is also where the SP metadata is served.
pub fn sp_entity_id(public_base_url: &str, connection_id: &str) -> String {
    format!(
        "{}/api/v1/auth/saml/{connection_id}/metadata",
        public_base_url.trim_end_matches('/')
    )
}

pub fn acs_url(public_base_url: &str) -> String {
    format!(
        "{}/api/v1/auth/saml/acs",
        public_base_url.trim_end_matches('/')
    )
}

/// Parses IdP metadata and refuses what we could not verify against: no
/// signing certificate, or no HTTP-Redirect sign-on endpoint. Returns the
/// IdP entity id.
pub fn validate_metadata(xml: &str) -> Result<String, AppError> {
    let sp = service_provider_for(xml, "https://placeholder.invalid", "check")?;
    if sp.sso_binding_location(HTTP_REDIRECT_BINDING).is_none() {
        return Err(AppError::ValidationError(
            "The IdP metadata has no HTTP-Redirect single sign-on endpoint".into(),
        ));
    }
    Ok(sp.idp_metadata.entity_id.clone().unwrap_or_default())
}

fn service_provider_for(
    metadata_xml: &str,
    public_base_url: &str,
    connection_id: &str,
) -> Result<ServiceProvider, AppError> {
    let metadata: EntityDescriptor = metadata_xml
        .parse()
        .map_err(|e| AppError::ValidationError(format!("Invalid IdP metadata: {e}")))?;
    let entity_id = sp_entity_id(public_base_url, connection_id);
    let sp = ServiceProviderBuilder::default()
        .entity_id(entity_id.clone())
        .metadata_url(entity_id)
        .acs_url(acs_url(public_base_url))
        .idp_metadata(metadata)
        .max_clock_skew(chrono::Duration::minutes(2))
        // Without this list samael accepts any algorithm (incl. SHA-1).
        .allowed_signature_algorithms(vec![
            AllowedSignatureAlgorithm::RsaSha256,
            AllowedSignatureAlgorithm::RsaSha384,
            AllowedSignatureAlgorithm::RsaSha512,
            AllowedSignatureAlgorithm::EcdsaSha256,
            AllowedSignatureAlgorithm::EcdsaSha384,
        ])
        .build()
        .map_err(|e| AppError::InternalError(e.to_string()))?;
    // samael only verifies signatures when the metadata names a signing
    // certificate; without one an unsigned response would be accepted.
    match sp.idp_signing_certs() {
        Ok(Some(certs)) if !certs.is_empty() => Ok(sp),
        _ => Err(AppError::ValidationError(
            "The IdP metadata has no signing certificate".into(),
        )),
    }
}

fn service_provider(
    connection: &Connection,
    public_base_url: &str,
) -> Result<ServiceProvider, AppError> {
    let xml = connection
        .idp_metadata_xml
        .as_deref()
        .ok_or_else(|| saml_error("this connection has no IdP metadata", ""))?;
    service_provider_for(xml, public_base_url, &connection.id)
}

/// SP metadata XML to give the IdP (entity id, ACS URL).
pub fn sp_metadata(connection: &Connection, public_base_url: &str) -> Result<String, AppError> {
    service_provider(connection, public_base_url)?
        .metadata()
        .and_then(|m| m.to_string().map_err(Into::into))
        .map_err(|e| AppError::InternalError(e.to_string()))
}

/// Builds the AuthnRequest (HTTP-Redirect) and stores its id, single use.
pub async fn start(
    state: &SharedState,
    connection: &Connection,
    email: &str,
    public_base_url: &str,
    forced: bool,
) -> Result<String, AppError> {
    let sp = service_provider(connection, public_base_url)?;
    let idp_url = sp
        .sso_binding_location(HTTP_REDIRECT_BINDING)
        .ok_or_else(|| saml_error("the IdP has no HTTP-Redirect endpoint", ""))?;
    let mut request = sp
        .make_authentication_request(&idp_url)
        .map_err(|e| saml_error("could not build the sign-in request", e))?;
    // samael's default id has 32 random bits; ours is 256.
    request.id = format!("id-{}", security::generate_token());
    if forced {
        // Checked on the way back against `AuthnInstant` (see `sso::gate`).
        request.force_authn = Some(true);
    }
    let relay_state = security::generate_token();

    let mut conn = state.db.acquire().await.map_err(db)?;
    save_login_state(
        &mut conn,
        &relay_state,
        &connection.id,
        &request.id,
        "",
        Some(email),
        forced,
    )
    .await?;
    request
        .redirect(&relay_state)
        .map_err(|e| saml_error("could not build the sign-in request", e))?
        .map(|url| url.to_string())
        .ok_or_else(|| saml_error("the sign-in request has no destination", ""))
}

/// ACS: consumes the relay state, verifies the response, provisions.
pub async fn finish(
    state: &SharedState,
    saml_response: &str,
    relay_state: &str,
    public_base_url: &str,
) -> Result<Finish, AppError> {
    let login = take_login_state(state, relay_state).await?;
    let (connection_id, request_id) = (login.connection_id.clone(), login.nonce.clone());
    let mut conn = state.db.acquire().await.map_err(db)?;
    let connection = connection(&mut conn, &connection_id)
        .await?
        .filter(|c| c.enabled && c.kind == "saml")
        .ok_or_else(|| saml_error("this connection is disabled", ""))?;
    drop(conn);

    let sp = service_provider(&connection, public_base_url)?;
    let assertion = sp
        .parse_base64_response(saml_response.trim(), Some(&[request_id.as_str()]))
        .map_err(|e| saml_error("the identity provider's response did not verify", e))?;
    let identity = identity_from(&assertion, &connection.groups_claim)?;

    let authenticated_at = assertion
        .authn_statements
        .iter()
        .flatten()
        .filter_map(|s| s.authn_instant)
        .max();
    match gate(
        &login,
        &identity.email,
        authenticated_at,
        chrono::Utc::now(),
    ) {
        Gate::Refuse => return Err(AppError::ValidationError(REAUTH_REFUSED.into())),
        Gate::Reauth => {
            let hint = login.login_hint.as_deref().unwrap_or(&identity.email);
            let redirect_url = start(state, &connection, hint, public_base_url, true).await?;
            return Ok(Finish::Reauth { redirect_url });
        }
        Gate::Proceed => {}
    }

    let mut tx = state.db.begin().await.map_err(db)?;
    let user_id = provision(&mut tx, &connection, &identity).await?;
    tx.commit().await.map_err(db)?;
    Ok(Finish::SignedIn {
        user_id,
        session: SsoSession {
            connection_id: connection.id.clone(),
            subject: identity.subject.clone(),
            ttl_minutes: connection.session_ttl_minutes,
            id_token_encrypted: None,
        },
    })
}

fn attribute_values<'a>(assertion: &'a Assertion, names: &[&str]) -> Vec<&'a str> {
    assertion
        .attribute_statements
        .iter()
        .flatten()
        .flat_map(|statement| &statement.attributes)
        .filter(|a| {
            a.name
                .as_deref()
                .is_some_and(|n| names.iter().any(|w| w.eq_ignore_ascii_case(n)))
        })
        .flat_map(|a| &a.values)
        .filter_map(|v| v.value.as_deref())
        .collect()
}

fn identity_from(assertion: &Assertion, groups_attribute: &str) -> Result<Identity, AppError> {
    let subject = assertion
        .subject
        .as_ref()
        .and_then(|s| s.name_id.as_ref())
        .map(|n| n.value.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| saml_error("the assertion has no NameID", ""))?;
    let email = attribute_values(assertion, EMAIL_ATTRIBUTES)
        .first()
        .map(|e| e.to_string())
        .or_else(|| subject.contains('@').then(|| subject.clone()))
        .ok_or_else(|| saml_error("the assertion carries no email address", ""))?;
    Ok(Identity {
        email,
        name: attribute_values(assertion, NAME_ATTRIBUTES)
            .first()
            .map(|n| n.to_string())
            .unwrap_or_default(),
        groups: attribute_values(assertion, &[groups_attribute])
            .into_iter()
            .map(str::to_string)
            .collect(),
        subject,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use base64::Engine;
    use samael::attribute::{Attribute, AttributeValue};
    use samael::crypto::{Crypto, CryptoProvider, mime_encode_x509_cert};
    use samael::idp::response_builder::build_response_template;
    use samael::idp::{CertificateParams, IdentityProvider, KeyType, Rsa};
    use samael::schema::AttributeStatement;
    use samael::signature::DigestAlgorithm;

    use super::*;
    use crate::ee::sso::provision::tests::seed_connection;
    use crate::testing::{cleanup_database, test_state};

    /// Entra's group attribute (what an admin puts in `groups_claim`).
    const ENTRA_GROUPS_ATTRIBUTE: &str =
        "http://schemas.microsoft.com/ws/2008/06/identity/claims/groups";
    const BASE: &str = "http://litepod.test";
    const IDP_ENTITY: &str = "https://idp.contoso.test";

    pub(crate) struct MockSamlIdp {
        idp: IdentityProvider,
        cert: samael::crypto::CertificateDer,
    }

    impl MockSamlIdp {
        pub(crate) fn new() -> Self {
            let idp = IdentityProvider::generate_new(KeyType::Rsa(Rsa::Rsa2048)).unwrap();
            let cert = idp
                .create_certificate(&CertificateParams {
                    common_name: "idp",
                    issuer_name: "idp",
                    days_until_expiration: 2,
                })
                .unwrap();
            MockSamlIdp { idp, cert }
        }

        pub(crate) fn metadata(&self, with_cert: bool) -> String {
            let key = if with_cert {
                format!(
                    r#"<md:KeyDescriptor use="signing"><ds:KeyInfo xmlns:ds="http://www.w3.org/2000/09/xmldsig#"><ds:X509Data><ds:X509Certificate>{}</ds:X509Certificate></ds:X509Data></ds:KeyInfo></md:KeyDescriptor>"#,
                    mime_encode_x509_cert(&self.cert)
                )
            } else {
                String::new()
            };
            format!(
                r#"<md:EntityDescriptor xmlns:md="urn:oasis:names:tc:SAML:2.0:metadata" entityID="{IDP_ENTITY}"><md:IDPSSODescriptor protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol">{key}<md:SingleSignOnService Binding="{HTTP_REDIRECT_BINDING}" Location="https://idp.contoso.test/sso"/></md:IDPSSODescriptor></md:EntityDescriptor>"#
            )
        }

        /// A signed response for `request_id` (SHA-256 digest unless `sha1`);
        /// `tamper` edits it after signing.
        pub(crate) fn response(
            &self,
            request_id: &str,
            audience: &str,
            groups: &[&str],
            tamper: bool,
            sha1: bool,
        ) -> String {
            self.response_at(
                request_id,
                audience,
                groups,
                tamper,
                sha1,
                chrono::Utc::now(),
            )
        }

        /// `response`, authenticated at `authn_instant`.
        pub(crate) fn response_at(
            &self,
            request_id: &str,
            audience: &str,
            groups: &[&str],
            tamper: bool,
            sha1: bool,
            authn_instant: chrono::DateTime<chrono::Utc>,
        ) -> String {
            let mut response = build_response_template(
                &self.cert,
                "ana-persistent-id",
                audience,
                IDP_ENTITY,
                &acs_url(BASE),
                request_id,
                &[],
            );
            let assertion = response.assertion.as_mut().unwrap();
            for statement in assertion.authn_statements.iter_mut().flatten() {
                statement.authn_instant = Some(authn_instant);
            }
            for confirmation in assertion
                .subject
                .as_mut()
                .unwrap()
                .subject_confirmations
                .as_mut()
                .unwrap()
            {
                confirmation
                    .subject_confirmation_data
                    .as_mut()
                    .unwrap()
                    .not_on_or_after = Some(chrono::Utc::now() + chrono::Duration::minutes(5));
            }
            let attribute = |name: &str, values: &[&str]| Attribute {
                friendly_name: None,
                name: Some(name.into()),
                name_format: None,
                values: values
                    .iter()
                    .map(|v| AttributeValue {
                        attribute_type: None,
                        value: Some(v.to_string()),
                    })
                    .collect(),
            };
            assertion.attribute_statements = Some(vec![AttributeStatement {
                attributes: vec![
                    attribute(EMAIL_ATTRIBUTES[0], &["ana@contoso.com"]),
                    attribute(NAME_ATTRIBUTES[0], &["Ana"]),
                    attribute(ENTRA_GROUPS_ATTRIBUTE, groups),
                ],
            }]);
            for reference in &mut response.signature.as_mut().unwrap().signed_info.reference {
                reference.digest_method.algorithm = if sha1 {
                    DigestAlgorithm::Sha1
                } else {
                    DigestAlgorithm::Sha256
                };
            }
            let xml = response.to_string().unwrap();
            let mut signed =
                Crypto::sign_xml(xml, &self.idp.export_private_key_der().unwrap()).unwrap();
            if tamper {
                signed = signed.replace("ana@contoso.com", "eve@contoso.com");
            }
            base64::engine::general_purpose::STANDARD.encode(signed)
        }
    }

    async fn setup(idp: &MockSamlIdp) -> (SharedState, std::path::PathBuf) {
        let (state, path) = test_state().await;
        seed_connection(&state).await;
        sqlx::query(
            "UPDATE sso_connections SET kind = 'saml', idp_metadata_xml = ?1, groups_claim = ?2",
        )
        .bind(idp.metadata(true))
        .bind(ENTRA_GROUPS_ATTRIBUTE)
        .execute(&state.db)
        .await
        .unwrap();
        (state, path)
    }

    /// (relay_state, request_id) as the IdP would see them.
    async fn begin(state: &SharedState) -> (String, String) {
        let url = crate::ee::sso::start_login(state, "ana@contoso.com", BASE, false)
            .await
            .unwrap();
        let url = url::Url::parse(&url).unwrap();
        assert!(url.query_pairs().any(|(k, _)| k == "SAMLRequest"));
        let relay = url
            .query_pairs()
            .find(|(k, _)| k == "RelayState")
            .map(|(_, v)| v.to_string())
            .unwrap();
        let request_id: String = sqlx::query_scalar("SELECT nonce FROM sso_login_states")
            .fetch_one(&state.db)
            .await
            .unwrap();
        (relay, request_id)
    }

    #[tokio::test]
    async fn a_signed_response_provisions_once() {
        let idp = MockSamlIdp::new();
        let (state, path) = setup(&idp).await;
        let (relay, request_id) = begin(&state).await;
        let response = idp.response(
            &request_id,
            &sp_entity_id(BASE, "c1"),
            &["g-ops"],
            false,
            false,
        );
        let user_id = crate::ee::sso::oidc::tests::signed_in(
            finish(&state, &response, &relay, BASE).await.unwrap(),
        );
        let in_group: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM rbac_group_members WHERE group_id = 'ops' AND user_id = ?1",
        )
        .bind(&user_id)
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(in_group, 1);
        assert!(
            finish(&state, &response, &relay, BASE).await.is_err(),
            "relay state is single use"
        );
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn tampered_wrong_audience_or_foreign_request_is_refused() {
        let idp = MockSamlIdp::new();
        let (state, path) = setup(&idp).await;
        let audience = sp_entity_id(BASE, "c1");

        let (relay, request_id) = begin(&state).await;
        let tampered = idp.response(&request_id, &audience, &[], true, false);
        assert!(
            finish(&state, &tampered, &relay, BASE).await.is_err(),
            "edited after signing"
        );

        let (relay, request_id) = begin_fresh(&state).await;
        let wrong_audience = idp.response(&request_id, "https://someone-else", &[], false, false);
        assert!(
            finish(&state, &wrong_audience, &relay, BASE).await.is_err(),
            "audience"
        );

        let (relay, _) = begin_fresh(&state).await;
        let foreign = idp.response("id-not-ours", &audience, &[], false, false);
        assert!(
            finish(&state, &foreign, &relay, BASE).await.is_err(),
            "InResponseTo"
        );

        let (relay, request_id) = begin_fresh(&state).await;
        let sha1 = idp.response(&request_id, &audience, &[], false, true);
        assert!(
            finish(&state, &sha1, &relay, BASE).await.is_err(),
            "SHA-1 digest"
        );
        cleanup_database(&path);
    }

    /// `begin` when earlier states may still be stored: picks the newest.
    async fn begin_fresh(state: &SharedState) -> (String, String) {
        sqlx::query("DELETE FROM sso_login_states")
            .execute(&state.db)
            .await
            .unwrap();
        begin(state).await
    }

    fn relay_of(url: &str) -> String {
        url::Url::parse(url)
            .unwrap()
            .query_pairs()
            .find(|(k, _)| k == "RelayState")
            .unwrap()
            .1
            .to_string()
    }

    async fn stored_forced(state: &SharedState) -> i64 {
        sqlx::query_scalar("SELECT forced FROM sso_login_states")
            .fetch_one(&state.db)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn forced_saml_logins_check_authn_instant() {
        let idp = MockSamlIdp::new();
        let (state, path) = setup(&idp).await;
        let url = crate::ee::sso::start_login(&state, "ana@contoso.com", BASE, true)
            .await
            .unwrap();
        assert_eq!(stored_forced(&state).await, 1);
        let request_id: String = sqlx::query_scalar("SELECT nonce FROM sso_login_states")
            .fetch_one(&state.db)
            .await
            .unwrap();
        let stale = idp.response_at(
            &request_id,
            &sp_entity_id(BASE, "c1"),
            &[],
            false,
            false,
            chrono::Utc::now() - chrono::Duration::hours(1),
        );
        let err = finish(&state, &stale, &relay_of(&url), BASE).await.err();
        assert!(
            matches!(&err, Some(AppError::ValidationError(m)) if m == crate::ee::sso::REAUTH_REFUSED),
            "{err:?}"
        );
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_saml_identity_other_than_typed_forces_credentials() {
        let idp = MockSamlIdp::new();
        let (state, path) = setup(&idp).await;
        let url = crate::ee::sso::start_login(&state, "eve@contoso.com", BASE, false)
            .await
            .unwrap();
        assert_eq!(stored_forced(&state).await, 0);
        let request_id: String = sqlx::query_scalar("SELECT nonce FROM sso_login_states")
            .fetch_one(&state.db)
            .await
            .unwrap();
        let response = idp.response(&request_id, &sp_entity_id(BASE, "c1"), &[], false, false);
        match finish(&state, &response, &relay_of(&url), BASE)
            .await
            .unwrap()
        {
            Finish::Reauth { redirect_url } => {
                assert!(redirect_url.contains("SAMLRequest="));
                assert_eq!(stored_forced(&state).await, 1, "re-asked with ForceAuthn");
            }
            Finish::SignedIn { .. } => panic!("must not sign in as ana"),
        }
        cleanup_database(&path);
    }

    #[test]
    fn metadata_without_a_signing_certificate_is_refused() {
        let idp = MockSamlIdp::new();
        assert!(validate_metadata(&idp.metadata(false)).is_err());
        assert_eq!(validate_metadata(&idp.metadata(true)).unwrap(), IDP_ENTITY);
    }
}
