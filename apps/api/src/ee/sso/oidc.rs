//! OIDC Authorization Code + PKCE (spec §2). `openidconnect` does discovery,
//! JWKS, PKCE and ID-token verification (signature, `iss`, `aud`, `exp`,
//! `nonce`); this module only persists the in-flight state and maps claims.

use base64::Engine;
use openidconnect::core::{CoreClient, CoreProviderMetadata, CoreResponseType};
use openidconnect::{
    AuthenticationFlow, AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce,
    OAuth2TokenResponse, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
};

use super::provision::{Identity, provision};
use super::{
    Connection, Finish, Gate, REAUTH_REFUSED, SsoSession, connection, db, gate, save_login_state,
    take_login_state,
};
use crate::dtos::response::AppError;
use crate::state::state::SharedState;
use crate::utils::security;

pub fn redirect_uri(public_base_url: &str) -> String {
    format!(
        "{}/api/v1/auth/sso/callback",
        public_base_url.trim_end_matches('/')
    )
}

fn http_client() -> Result<openidconnect::reqwest::Client, AppError> {
    // No redirects: an IdP endpoint that redirects is misconfigured, and
    // following it would let it point us anywhere (SSRF).
    openidconnect::reqwest::Client::builder()
        .redirect(openidconnect::reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| AppError::InternalError(e.to_string()))
}

fn idp_error(context: &str, e: impl std::fmt::Display) -> AppError {
    tracing::warn!(error = %e, "sso: {context}");
    AppError::ValidationError(format!("Single sign-on failed: {context}"))
}

async fn metadata(
    connection: &Connection,
    http: &openidconnect::reqwest::Client,
) -> Result<CoreProviderMetadata, AppError> {
    let issuer = connection
        .issuer
        .as_deref()
        .ok_or_else(|| idp_error("connection has no issuer", ""))?;
    CoreProviderMetadata::discover_async(
        IssuerUrl::new(issuer.to_string()).map_err(|e| idp_error("invalid issuer", e))?,
        http,
    )
    .await
    .map_err(|e| idp_error("could not read the identity provider's configuration", e))
}

fn client_secret(connection: &Connection) -> Result<Option<ClientSecret>, AppError> {
    connection
        .client_secret_encrypted
        .as_deref()
        .map(|encrypted| {
            security::decrypt_data(encrypted, &security::get_encryption_key())
                .map(ClientSecret::new)
                .map_err(|e| AppError::InternalError(e))
        })
        .transpose()
}

/// RP-initiated logout URL for this connection, or `None` when the IdP has no
/// `end_session_endpoint` or cannot be reached. Never fails: logout must not.
/// `openidconnect`'s core metadata does not model the endpoint, so the raw
/// discovery document is read.
pub async fn end_session_url(
    connection: &Connection,
    id_token_encrypted: Option<&str>,
    public_base_url: &str,
) -> Option<String> {
    let issuer = connection.issuer.as_deref()?.trim_end_matches('/');
    let http = http_client().ok()?;
    let discovery: serde_json::Value = http
        .get(format!("{issuer}/.well-known/openid-configuration"))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()?;
    let mut url = url::Url::parse(discovery.get("end_session_endpoint")?.as_str()?).ok()?;
    {
        let mut query = url.query_pairs_mut();
        if let Some(id_token) = id_token_encrypted
            .and_then(|e| security::decrypt_data(e, &security::get_encryption_key()).ok())
        {
            query.append_pair("id_token_hint", &id_token);
        }
        if let Some(client_id) = connection.client_id.as_deref() {
            query.append_pair("client_id", client_id);
        }
        query.append_pair(
            "post_logout_redirect_uri",
            &format!("{}/login", public_base_url.trim_end_matches('/')),
        );
    }
    Some(url.to_string())
}

const BACKCHANNEL_EVENT: &str = "http://schemas.openid.net/event/backchannel-logout";

#[derive(serde::Deserialize)]
struct LogoutClaims {
    sub: Option<String>,
    iat: i64,
    jti: Option<String>,
    events: serde_json::Map<String, serde_json::Value>,
    nonce: Option<serde_json::Value>,
}

/// OIDC Back-Channel Logout 1.0 §2.6. Returns the `sub` to sign out.
/// Every failure is the same generic error: the caller answers 400.
pub async fn validate_logout_token(
    state: &SharedState,
    connection: &Connection,
    token: &str,
) -> Result<String, AppError> {
    let invalid = |why: &str| idp_error("invalid logout token", why);
    let http = http_client()?;
    let metadata = metadata(connection, &http).await?;
    let jwks: jsonwebtoken::jwk::JwkSet = http
        .get(metadata.jwks_uri().url().as_str())
        .send()
        .await
        .map_err(|e| invalid(&e.to_string()))?
        .json()
        .await
        .map_err(|e| invalid(&e.to_string()))?;

    let header = jsonwebtoken::decode_header(token).map_err(|e| invalid(&e.to_string()))?;
    let jwk = match header.kid.as_deref() {
        Some(kid) => jwks.find(kid),
        None => jwks.keys.first(),
    }
    .ok_or_else(|| invalid("unknown signing key"))?;
    let key = jsonwebtoken::DecodingKey::from_jwk(jwk).map_err(|e| invalid(&e.to_string()))?;

    // `alg: none` cannot get here: jsonwebtoken has no such algorithm.
    let mut validation = jsonwebtoken::Validation::new(header.alg);
    validation.set_issuer(&[connection.issuer.as_deref().unwrap_or_default()]);
    validation.set_audience(&[connection.client_id.as_deref().unwrap_or_default()]);
    // `exp` is checked when present; `iat` is checked below.
    validation.required_spec_claims = ["iss", "aud"].into_iter().map(String::from).collect();
    validation.leeway = 60;
    let claims = jsonwebtoken::decode::<LogoutClaims>(token, &key, &validation)
        .map_err(|e| invalid(&e.to_string()))?
        .claims;

    if claims.nonce.is_some() {
        return Err(invalid("nonce present"));
    }
    if !claims.events.contains_key(BACKCHANNEL_EVENT) {
        return Err(invalid("missing back-channel event"));
    }
    if (chrono::Utc::now().timestamp() - claims.iat).abs() > 300 {
        return Err(invalid("iat out of range"));
    }
    let sub = claims
        .sub
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("no sub (turn off 'session required' in the IdP)"))?;
    // Replay guard, best-effort: replaying only deletes sessions again.
    if let Some(jti) = claims.jti {
        let key = format!("sso:backchannel:jti:{}:{jti}", connection.id);
        if let Ok(false) = state.ephemeral.set_json_if_absent(&key, &true, 600).await {
            return Err(invalid("replayed jti"));
        }
    }
    Ok(sub)
}

/// Spec §2 start for an OIDC connection: returns the IdP authorize URL.
pub async fn start(
    state: &SharedState,
    connection: &Connection,
    email: &str,
    public_base_url: &str,
    forced: bool,
) -> Result<String, AppError> {
    let mut conn = state.db.acquire().await.map_err(db)?;
    let http = http_client()?;
    let client = CoreClient::from_provider_metadata(
        metadata(&connection, &http).await?,
        ClientId::new(connection.client_id.clone().unwrap_or_default()),
        client_secret(&connection)?,
    )
    .set_redirect_uri(
        RedirectUrl::new(redirect_uri(public_base_url)).map_err(|e| idp_error("redirect", e))?,
    );

    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let mut request = client
        .authorize_url(
            AuthenticationFlow::<CoreResponseType>::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .add_scope(Scope::new("email".into()))
        .add_scope(Scope::new("profile".into()))
        .set_pkce_challenge(challenge)
        // Lets the IdP pre-fill the account picker.
        .add_extra_param("login_hint", email.trim());
    if forced {
        // `max_age=0` makes `auth_time` REQUIRED in the ID token (OIDC Core
        // §3.1.2.1), which `gate` checks: an IdP that ignores `prompt` is caught.
        request = request
            .add_extra_param("prompt", "login")
            .add_extra_param("max_age", "0");
    }
    let (url, csrf, nonce) = request.url();

    save_login_state(
        &mut conn,
        csrf.secret(),
        &connection.id,
        nonce.secret(),
        verifier.secret(),
        Some(email),
        forced,
    )
    .await?;
    Ok(url.to_string())
}

/// Spec §2 callback: consumes the state (single use), exchanges the code,
/// verifies the ID token, applies `gate`, and provisions.
pub async fn finish(
    state: &SharedState,
    code: &str,
    state_param: &str,
    public_base_url: &str,
) -> Result<Finish, AppError> {
    let login = take_login_state(state, state_param).await?;
    let (connection_id, nonce, verifier) = (
        login.connection_id.clone(),
        login.nonce.clone(),
        login.pkce_verifier.clone(),
    );

    let mut conn = state.db.acquire().await.map_err(db)?;
    let connection = connection(&mut conn, &connection_id)
        .await?
        .filter(|c| c.enabled && c.kind == "oidc")
        .ok_or_else(|| idp_error("this connection is disabled", ""))?;
    drop(conn);

    let http = http_client()?;
    let client = CoreClient::from_provider_metadata(
        metadata(&connection, &http).await?,
        ClientId::new(connection.client_id.clone().unwrap_or_default()),
        client_secret(&connection)?,
    )
    .set_redirect_uri(
        RedirectUrl::new(redirect_uri(public_base_url)).map_err(|e| idp_error("redirect", e))?,
    );
    let tokens = client
        .exchange_code(AuthorizationCode::new(code.to_string()))
        .map_err(|e| idp_error("the provider has no token endpoint", e))?
        .set_pkce_verifier(PkceCodeVerifier::new(verifier))
        .request_async(&http)
        .await
        .map_err(|e| idp_error("the code exchange was refused", e))?;
    let id_token = tokens
        .id_token()
        .ok_or_else(|| idp_error("the provider sent no ID token", ""))?;
    let claims = id_token
        .claims(&client.id_token_verifier(), &Nonce::new(nonce))
        .map_err(|e| idp_error("the ID token did not verify", e))?;

    // The signature is verified above, so the raw payload can be trusted for
    // the claims `openidconnect`'s core types do not model (groups, overage).
    let payload = jwt_payload(&id_token.to_string())?;
    let email = claims
        .email()
        .map(|e| e.as_str().to_string())
        .or_else(|| {
            claims
                .preferred_username()
                .map(|u| u.as_str().to_string())
                .filter(|u| u.contains('@'))
        })
        .ok_or_else(|| idp_error("the ID token carries no email address", ""))?;
    let groups = match groups_from(&payload, &connection.groups_claim) {
        Some(groups) => groups,
        None if has_group_overage(&payload) => {
            graph_member_groups(&http, tokens.access_token().secret()).await?
        }
        None => Vec::new(),
    };
    let identity = Identity {
        subject: claims.subject().as_str().to_string(),
        email,
        name: claims
            .name()
            .and_then(|n| n.get(None))
            .map(|n| n.as_str().to_string())
            .unwrap_or_default(),
        groups,
    };

    match gate(
        &login,
        &identity.email,
        claims.auth_time(),
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
    // Only used as `id_token_hint` at logout; a failure here must not block sign-in.
    let id_token_encrypted =
        security::encrypt_data(&id_token.to_string(), &security::get_encryption_key()).ok();

    let mut tx = state.db.begin().await.map_err(db)?;
    let user_id = provision(&mut tx, &connection, &identity).await?;
    tx.commit().await.map_err(db)?;
    Ok(Finish::SignedIn {
        user_id,
        session: SsoSession {
            connection_id: connection.id.clone(),
            subject: identity.subject.clone(),
            ttl_minutes: connection.session_ttl_minutes,
            id_token_encrypted,
        },
    })
}

fn jwt_payload(jwt: &str) -> Result<serde_json::Value, AppError> {
    let part = jwt
        .split('.')
        .nth(1)
        .ok_or_else(|| idp_error("malformed ID token", ""))?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(part)
        .map_err(|e| idp_error("malformed ID token", e))?;
    serde_json::from_slice(&bytes).map_err(|e| idp_error("malformed ID token", e))
}

fn groups_from(payload: &serde_json::Value, claim: &str) -> Option<Vec<String>> {
    payload.get(claim)?.as_array().map(|values| {
        values
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    })
}

/// Entra omits `groups` past 200 and points at Graph instead (`_claim_names`).
fn has_group_overage(payload: &serde_json::Value) -> bool {
    payload
        .get("_claim_names")
        .and_then(|names| names.get("groups"))
        .is_some()
}

/// Spec §2 step 5. Needs delegated `GroupMember.Read.All` with admin consent;
/// without it the login fails loudly rather than silently dropping groups.
async fn graph_member_groups(
    http: &openidconnect::reqwest::Client,
    access_token: &str,
) -> Result<Vec<String>, AppError> {
    let response = http
        .post("https://graph.microsoft.com/v1.0/me/getMemberGroups")
        .bearer_auth(access_token)
        .json(&serde_json::json!({ "securityEnabledOnly": true }))
        .send()
        .await
        .map_err(|e| idp_error("could not read your groups from Microsoft Graph", e))?;
    if !response.status().is_success() {
        return Err(idp_error(
            "Microsoft Graph refused the group lookup (grant GroupMember.Read.All)",
            response.status(),
        ));
    }
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| idp_error("unreadable Microsoft Graph response", e))?;
    Ok(groups_from(&body, "value").unwrap_or_default())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};

    use axum::extract::{Form, State};
    use axum::{Json, Router, routing::get, routing::post};
    use jsonwebtoken::{Algorithm, EncodingKey, Header};
    use openssl::rsa::Rsa;
    use serde_json::{Value, json};

    use super::*;
    use crate::ee::sso::provision::tests::seed_connection;
    use crate::testing::{cleanup_database, test_state};

    /// What the next ID token will say; tests flip fields to probe the verifier.
    #[derive(Clone)]
    pub(crate) struct MockIdp {
        pub issuer: String,
        pub claims: Arc<Mutex<Value>>,
        key: Arc<EncodingKey>,
        /// Signs with a different key than the JWKS publishes.
        pub forge: Arc<Mutex<bool>>,
        other_key: Arc<EncodingKey>,
    }

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    /// Discovery, JWKS and a token endpoint on 127.0.0.1, signing RS256.
    pub(crate) async fn mock_idp() -> MockIdp {
        let rsa = Rsa::generate(2048).unwrap();
        let jwk = json!({"keys": [{
            "kty": "RSA", "use": "sig", "alg": "RS256", "kid": "k1",
            "n": b64(&rsa.n().to_vec()), "e": b64(&rsa.e().to_vec()),
        }]});
        let key = EncodingKey::from_rsa_pem(&rsa.private_key_to_pem().unwrap()).unwrap();
        let other = Rsa::generate(2048).unwrap();
        let other_key = EncodingKey::from_rsa_pem(&other.private_key_to_pem().unwrap()).unwrap();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        let issuer = format!("http://{addr}");
        let idp = MockIdp {
            issuer: issuer.clone(),
            claims: Arc::new(Mutex::new(json!({}))),
            key: Arc::new(key),
            forge: Arc::new(Mutex::new(false)),
            other_key: Arc::new(other_key),
        };

        let discovery = json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{issuer}/authorize"),
            "token_endpoint": format!("{issuer}/token"),
            "jwks_uri": format!("{issuer}/jwks"),
            "end_session_endpoint": format!("{issuer}/logout"),
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256"],
        });
        let app = Router::new()
            .route(
                "/.well-known/openid-configuration",
                get(move || {
                    let d = discovery.clone();
                    async move { Json(d) }
                }),
            )
            .route(
                "/jwks",
                get(move || {
                    let j = jwk.clone();
                    async move { Json(j) }
                }),
            )
            .route("/token", post(token))
            .with_state(idp.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        idp
    }

    async fn token(
        State(idp): State<MockIdp>,
        Form(_form): Form<std::collections::HashMap<String, String>>,
    ) -> Json<Value> {
        let claims = idp.claims.lock().unwrap().clone();
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("k1".into());
        let key = if *idp.forge.lock().unwrap() {
            idp.other_key.clone()
        } else {
            idp.key.clone()
        };
        let id_token = jsonwebtoken::encode(&header, &claims, &key).unwrap();
        Json(json!({
            "access_token": "at", "token_type": "Bearer", "expires_in": 3600, "id_token": id_token,
        }))
    }

    impl MockIdp {
        /// A JWT over `claims` with the published key (as a back-channel IdP sends).
        pub(crate) fn sign(&self, claims: &Value) -> String {
            let mut header = Header::new(Algorithm::RS256);
            header.kid = Some("k1".into());
            jsonwebtoken::encode(&header, claims, &self.key).unwrap()
        }

        /// A valid logout token's claims; a `null` override removes the claim.
        pub(crate) fn logout_claims(&self, overrides: Value) -> Value {
            let now = chrono::Utc::now().timestamp();
            let mut claims = json!({
                "iss": self.issuer, "aud": "litepod", "sub": "sub-ana", "iat": now, "exp": now + 120,
                "jti": uuid::Uuid::now_v7().to_string(),
                "events": { "http://schemas.openid.net/event/backchannel-logout": {} },
            });
            for (k, v) in overrides.as_object().unwrap() {
                if v.is_null() {
                    claims.as_object_mut().unwrap().remove(k);
                } else {
                    claims[k] = v.clone();
                }
            }
            claims
        }

        /// Claims for a good login; `nonce` must be the one `start` stored.
        pub(crate) fn set(&self, nonce: &str, overrides: Value) {
            let now = chrono::Utc::now().timestamp();
            let mut claims = json!({
                "iss": self.issuer, "aud": "litepod", "sub": "sub-ana",
                "email": "ana@contoso.com", "name": "Ana",
                "iat": now, "exp": now + 300, "nonce": nonce, "groups": ["g-ops"],
            });
            for (k, v) in overrides.as_object().unwrap() {
                claims[k] = v.clone();
            }
            *self.claims.lock().unwrap() = claims;
        }
    }

    /// Runs `start` and returns (state, nonce) as the IdP would receive them.
    pub(crate) async fn begin(state: &SharedState) -> (String, String) {
        begin_as(state, "ana@contoso.com", false).await.0
    }

    type Params = std::collections::HashMap<String, String>;

    /// Like `begin`, also returning the authorize URL's query pairs.
    pub(crate) async fn begin_as(
        state: &SharedState,
        email: &str,
        forced: bool,
    ) -> ((String, String), Params) {
        let url = crate::ee::sso::start_login(state, email, "http://litepod.test", forced)
            .await
            .unwrap();
        authorize_params(&url)
    }

    fn authorize_params(url: &str) -> ((String, String), Params) {
        let url = url::Url::parse(url).unwrap();
        let params: Params = url.query_pairs().into_owned().collect();
        assert_eq!(params["code_challenge_method"], "S256");
        ((params["state"].clone(), params["nonce"].clone()), params)
    }

    pub(crate) fn signed_in(outcome: Finish) -> String {
        match outcome {
            Finish::SignedIn { user_id, .. } => user_id,
            Finish::Reauth { .. } => panic!("expected SignedIn, got Reauth"),
        }
    }

    pub(crate) async fn setup() -> (SharedState, std::path::PathBuf, MockIdp) {
        let (state, path) = test_state().await;
        seed_connection(&state).await;
        let idp = mock_idp().await;
        sqlx::query("UPDATE sso_connections SET issuer = ?1")
            .bind(&idp.issuer)
            .execute(&state.db)
            .await
            .unwrap();
        (state, path, idp)
    }

    #[tokio::test]
    async fn a_good_login_provisions_and_the_state_is_single_use() {
        let (state, path, idp) = setup().await;
        let (csrf, nonce) = begin(&state).await;
        idp.set(&nonce, json!({}));
        let user_id = signed_in(
            finish(&state, "code", &csrf, "http://litepod.test")
                .await
                .unwrap(),
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
            finish(&state, "code", &csrf, "http://litepod.test")
                .await
                .is_err(),
            "replayed state"
        );
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn the_id_token_is_verified() {
        let (state, path, idp) = setup().await;
        for (case, overrides, forge) in [
            ("wrong audience", json!({"aud": "someone-else"}), false),
            ("expired", json!({"exp": 1_000_000}), false),
            (
                "wrong issuer",
                json!({"iss": "https://evil.example"}),
                false,
            ),
            ("bad signature", json!({}), true),
        ] {
            let (csrf, nonce) = begin(&state).await;
            idp.set(&nonce, overrides);
            *idp.forge.lock().unwrap() = forge;
            assert!(
                finish(&state, "code", &csrf, "http://litepod.test")
                    .await
                    .is_err(),
                "{case} must be refused"
            );
        }
        // Nonce from a different login.
        let (csrf, _) = begin(&state).await;
        *idp.forge.lock().unwrap() = false;
        idp.set("not-the-nonce", json!({}));
        assert!(
            finish(&state, "code", &csrf, "http://litepod.test")
                .await
                .is_err()
        );
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn an_unconfigured_domain_looks_like_a_wrong_password() {
        let (state, path, _idp) = setup().await;
        assert!(matches!(
            crate::ee::sso::start_login(
                &state,
                "someone@unknown.example",
                "http://litepod.test",
                false
            )
            .await,
            Err(AppError::InvalidCredentials)
        ));
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_different_identity_than_typed_forces_credentials() {
        let (state, path, idp) = setup().await;
        // Someone types eve@ while the IdP still holds ana's session.
        let ((csrf, nonce), params) = begin_as(&state, "eve@contoso.com", false).await;
        assert!(!params.contains_key("prompt"));
        idp.set(&nonce, json!({}));
        let redirect_url = match finish(&state, "code", &csrf, "http://litepod.test")
            .await
            .unwrap()
        {
            Finish::Reauth { redirect_url } => redirect_url,
            Finish::SignedIn { .. } => panic!("must not sign in as ana"),
        };
        let users: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE email = 'ana@contoso.com'")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(users, 0, "nothing provisioned");

        let ((csrf, nonce), params) = authorize_params(&redirect_url);
        assert_eq!(params["prompt"], "login");
        assert_eq!(params["max_age"], "0");
        assert_eq!(params["login_hint"], "eve@contoso.com");

        // The IdP really asked: fresh auth_time, accepted whoever it is.
        idp.set(
            &nonce,
            json!({ "auth_time": chrono::Utc::now().timestamp() }),
        );
        signed_in(
            finish(&state, "code", &csrf, "http://litepod.test")
                .await
                .unwrap(),
        );
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn a_forced_login_without_fresh_auth_time_is_refused() {
        let (state, path, idp) = setup().await;
        for (case, overrides) in [
            (
                "IdP ignored prompt",
                json!({ "auth_time": chrono::Utc::now().timestamp() - 3600 }),
            ),
            ("no auth_time", json!({})),
        ] {
            let ((csrf, nonce), _) = begin_as(&state, "ana@contoso.com", true).await;
            idp.set(&nonce, overrides);
            let err = finish(&state, "code", &csrf, "http://litepod.test")
                .await
                .err();
            assert!(
                matches!(&err, Some(AppError::ValidationError(m)) if m == crate::ee::sso::REAUTH_REFUSED),
                "{case}: {err:?}"
            );
        }
        let users: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE email = 'ana@contoso.com'")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(users, 0);
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn signed_in_carries_the_session_origin() {
        let (state, path, idp) = setup().await;
        sqlx::query("UPDATE sso_connections SET session_ttl_minutes = 60")
            .execute(&state.db)
            .await
            .unwrap();
        let ((csrf, nonce), _) = begin_as(&state, "ANA@contoso.com", false).await;
        idp.set(&nonce, json!({}));
        match finish(&state, "code", &csrf, "http://litepod.test")
            .await
            .unwrap()
        {
            Finish::SignedIn { session, .. } => {
                assert_eq!(session.connection_id, "c1");
                assert_eq!(session.subject, "sub-ana");
                assert_eq!(session.ttl_minutes, 60);
                let raw = security::decrypt_data(
                    session.id_token_encrypted.as_deref().unwrap(),
                    &security::get_encryption_key(),
                )
                .unwrap();
                assert_eq!(raw.split('.').count(), 3, "a JWT");
            }
            Finish::Reauth { .. } => panic!("case-only difference must not force re-auth"),
        }
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn end_session_url_carries_hint_client_and_return_uri() {
        let (state, path, idp) = setup().await;
        let mut conn = state.db.acquire().await.unwrap();
        let connection = crate::ee::sso::connection(&mut conn, "c1")
            .await
            .unwrap()
            .unwrap();
        drop(conn);
        let enc = security::encrypt_data("a.b.c", &security::get_encryption_key()).unwrap();

        let url = end_session_url(&connection, Some(&enc), "http://litepod.test")
            .await
            .unwrap();
        let url = url::Url::parse(&url).unwrap();
        assert_eq!(
            url.as_str().split('?').next().unwrap(),
            format!("{}/logout", idp.issuer)
        );
        let p: Params = url.query_pairs().into_owned().collect();
        assert_eq!(p["id_token_hint"], "a.b.c");
        assert_eq!(p["client_id"], "litepod");
        assert_eq!(p["post_logout_redirect_uri"], "http://litepod.test/login");

        // Undecryptable token: still a URL, just without the hint.
        let url = end_session_url(&connection, Some("garbage"), "http://litepod.test")
            .await
            .unwrap();
        assert!(!url.contains("id_token_hint"));

        // IdP unreachable: None (the caller falls back to the reauth cookie).
        let mut down = connection.clone();
        down.issuer = Some("http://127.0.0.1:9".into());
        assert!(
            end_session_url(&down, None, "http://litepod.test")
                .await
                .is_none()
        );
        cleanup_database(&path);
    }

    #[tokio::test]
    async fn logout_tokens_are_validated() {
        let (state, path, idp) = setup().await;
        let mut conn = state.db.acquire().await.unwrap();
        let connection = crate::ee::sso::connection(&mut conn, "c1")
            .await
            .unwrap()
            .unwrap();
        drop(conn);

        let good = idp.sign(&idp.logout_claims(json!({})));
        assert_eq!(
            validate_logout_token(&state, &connection, &good)
                .await
                .unwrap(),
            "sub-ana"
        );
        // The `jti` replay guard needs Dragonfly, absent in tests; it is
        // best-effort by design (a replay only deletes sessions again).

        let old = chrono::Utc::now().timestamp() - 3600;
        for (case, overrides) in [
            ("nonce present", json!({"nonce": "n"})),
            ("wrong audience", json!({"aud": "other"})),
            ("wrong issuer", json!({"iss": "https://evil.example"})),
            ("no event", json!({"events": {}})),
            ("no sub", json!({"sub": null})),
            ("old iat", json!({"iat": old, "exp": null})),
            ("expired", json!({"exp": old})),
        ] {
            let token = idp.sign(&idp.logout_claims(overrides));
            assert!(
                validate_logout_token(&state, &connection, &token)
                    .await
                    .is_err(),
                "{case}"
            );
        }
        *idp.forge.lock().unwrap() = true;
        let forged = {
            let mut header = Header::new(Algorithm::RS256);
            header.kid = Some("k1".into());
            jsonwebtoken::encode(&header, &idp.logout_claims(json!({})), &idp.other_key).unwrap()
        };
        assert!(
            validate_logout_token(&state, &connection, &forged)
                .await
                .is_err(),
            "bad signature"
        );
        assert!(
            validate_logout_token(&state, &connection, "not.a.jwt")
                .await
                .is_err()
        );
        cleanup_database(&path);
    }
}
