use s3::Bucket;
use s3::creds::Credentials;
use s3::region::Region;

use crate::backup::config::S3Config;
use crate::dtos::response::AppError;

/// Builds a path-style S3 client for the configured destination. Path-style
/// (not virtual-hosted) is required for most self-hosted S3-compatible
/// stores (MinIO, etc.) that do not support bucket-subdomain DNS.
pub fn client(config: &S3Config) -> Result<Box<Bucket>, AppError> {
    let region = Region::Custom {
        region: config.region.clone(),
        endpoint: config.endpoint_url.clone(),
    };
    let credentials = Credentials::new(
        Some(&config.access_key_id),
        Some(&config.secret_access_key),
        None,
        None,
        None,
    )
    .map_err(|error| AppError::ValidationError(format!("Invalid S3 credentials: {error}")))?;
    let bucket = Bucket::new(&config.bucket, region, credentials)
        .map_err(|error| AppError::ValidationError(format!("Could not build S3 client: {error}")))?
        .with_path_style();
    Ok(bucket)
}

/// Lightweight reachability check run before persisting S3 settings, so a
/// typo'd endpoint/bucket/credential fails fast instead of surfacing only
/// during the next backup.
pub async fn test_connection(config: &S3Config) -> Result<(), AppError> {
    let bucket = client(config)?;
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        bucket.list(String::new(), Some("/".into())),
    )
    .await
    .map_err(|_| AppError::ValidationError("Timed out reaching S3 endpoint".into()))?
    .map_err(|error| AppError::ValidationError(format!("Could not reach S3 bucket: {error}")))?;
    Ok(())
}

/// Backup archives under `backups/`, newest first. Keys start with a
/// `%Y-%m-%dT%H-%M-%SZ` stamp, so reverse key order is reverse time order.
pub async fn list_backups(config: &S3Config) -> Result<Vec<(String, u64)>, AppError> {
    let bucket = client(config)?;
    let pages = bucket
        .list("backups/".to_string(), None)
        .await
        .map_err(|error| {
            AppError::ValidationError(format!("Could not list S3 backups: {error}"))
        })?;
    let mut backups: Vec<(String, u64)> = pages
        .into_iter()
        .flat_map(|page| page.contents)
        .filter(|object| object.key.ends_with(".tar"))
        .map(|object| (object.key, object.size))
        .collect();
    backups.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(backups)
}
