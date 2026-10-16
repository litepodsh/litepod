use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Legacy preset scopes. A token may also name catalog permissions directly
/// (see `authz::permission::token_grantable`). Deliberately no `secrets` scope:
/// no endpoint returns a decrypted secret today, so a scope gating one would
/// protect nothing that `write` doesn't already gate — add one the day such
/// an endpoint exists.
pub const VALID_SCOPES: [&str; 3] = ["read", "write", "deploy"];

/// Longest a token may live from the moment it's created.
pub const MAX_EXPIRY_DAYS: i64 = 365;

#[derive(Deserialize, ToSchema)]
pub struct CreateTokenRequest {
    pub name: String,
    pub scopes: Vec<String>,
    /// `None` = never expires. When set, must be in the future and at most
    /// `MAX_EXPIRY_DAYS` out.
    pub expires_at: Option<NaiveDate>,
}

#[derive(Serialize, ToSchema)]
pub struct PersonalAccessTokenDto {
    pub id: String,
    pub name: String,
    /// e.g. `lp_1a2b3c4d5e` — enough to recognize the token in a list, never
    /// enough to reconstruct it.
    pub token_prefix: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub created_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct CreatedTokenDto {
    pub token_info: PersonalAccessTokenDto,
    /// The raw token. Returned exactly once, on creation — never again.
    pub token: String,
}
