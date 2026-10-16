use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CatalogPortDto {
    pub container: u16,
    pub description: String,
    pub primary: bool,
    /// Compose service that owns the port, when several services expose it.
    pub service: Option<String>,
}
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CatalogFileDto {
    pub filename: String,
    pub content: String,
}
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CatalogTemplateDto {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub categories: Vec<String>,
    pub tags: Vec<String>,
    pub ports: Vec<CatalogPortDto>,
    pub icon_url: Option<String>,
    pub ui_url: Option<String>,
    pub website: Option<String>,
    pub docs: Option<String>,
    pub source: Option<String>,
    pub license: Option<String>,
    /// The app does not fully work over plain http (WebAuthn, browser crypto, Secure cookies).
    pub requires_https: bool,
    /// The app stores its URL internally on first boot; changing the env alone does not move it.
    pub url_persisted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<CatalogFileDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compose: Option<CatalogFileDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<CatalogFileDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup: Option<CatalogFileDto>,
}
