use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::Rng;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::dtos::response::AppError;
use crate::state::state::SharedState;

const KEY_PURPOSE: &[u8] = b"litepod.backup.s3secret.v1";

/// Full S3 destination config with the secret decrypted, for internal use
/// (building the S3 client, running a connectivity check). Never serialized.
#[derive(Clone)]
pub struct S3Config {
    pub endpoint_url: String,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub schedule_enabled: bool,
    pub schedule_interval: Option<String>,
    pub schedule_time: Option<String>,
}

/// Client-facing shape: the secret is never returned, only whether one is set.
#[derive(Debug, Serialize)]
pub struct S3ConfigDto {
    pub endpoint_url: String,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    pub has_secret_key: bool,
    pub schedule_enabled: bool,
    pub schedule_interval: Option<String>,
    pub schedule_time: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateS3ConfigRequest {
    pub endpoint_url: String,
    pub bucket: String,
    pub region: String,
    pub access_key_id: String,
    /// `None` keeps the existing encrypted secret. `Some("")` is rejected —
    /// an S3 config with no secret at all is never a valid persisted state.
    #[serde(default)]
    pub secret_access_key: Option<String>,
    #[serde(default)]
    pub schedule_enabled: bool,
    #[serde(default)]
    pub schedule_interval: Option<String>,
    #[serde(default)]
    pub schedule_time: Option<String>,
}

fn master_key() -> [u8; 32] {
    use sha2::Digest;
    let base = crate::utils::security::get_encryption_key();
    let mut hasher = sha2::Sha256::new();
    hasher.update(base);
    hasher.update(KEY_PURPOSE);
    let result = hasher.finalize();
    let mut master = [0u8; 32];
    master.copy_from_slice(&result);
    master
}

pub(crate) struct Encrypted {
    pub(crate) wrapped_dek: Vec<u8>,
    pub(crate) dek_nonce: Vec<u8>,
    pub(crate) ciphertext: Vec<u8>,
    pub(crate) nonce: Vec<u8>,
}

fn encrypt(master: &[u8; 32], plaintext: &[u8]) -> Result<Encrypted, String> {
    let mut dek = [0u8; 32];
    rand::rng().fill_bytes(&mut dek);
    let mut dek_nonce_bytes = [0u8; 12];
    rand::rng().fill_bytes(&mut dek_nonce_bytes);
    let mut nonce_bytes = [0u8; 12];
    rand::rng().fill_bytes(&mut nonce_bytes);

    let dek_cipher =
        Aes256Gcm::new_from_slice(&dek).map_err(|e| format!("dek cipher init: {e}"))?;
    let ciphertext = dek_cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext)
        .map_err(|e| format!("dek encrypt failed: {e}"))?;

    let master_cipher =
        Aes256Gcm::new_from_slice(master).map_err(|e| format!("master cipher init: {e}"))?;
    let wrapped_dek = master_cipher
        .encrypt(Nonce::from_slice(&dek_nonce_bytes), dek.as_ref())
        .map_err(|e| format!("master encrypt failed: {e}"))?;

    Ok(Encrypted {
        wrapped_dek,
        dek_nonce: dek_nonce_bytes.to_vec(),
        ciphertext,
        nonce: nonce_bytes.to_vec(),
    })
}

fn decrypt(
    master: &[u8; 32],
    wrapped_dek: &[u8],
    dek_nonce: &[u8],
    ciphertext: &[u8],
    nonce: &[u8],
) -> Result<Vec<u8>, String> {
    let master_cipher =
        Aes256Gcm::new_from_slice(master).map_err(|_| "master cipher init failed".to_string())?;
    let mut dek = master_cipher
        .decrypt(Nonce::from_slice(dek_nonce), wrapped_dek)
        .map_err(|_| "master decrypt failed".to_string())?;
    if dek.len() != 32 {
        dek.zeroize();
        return Err("dek length invalid".into());
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&dek);
    dek.zeroize();

    let dek_cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|_| "dek cipher init failed".to_string())?;
    let plaintext = dek_cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| "dek decrypt failed".to_string())?;
    key.zeroize();
    Ok(plaintext)
}

/// Encrypts a secret for at-rest storage (used by both the global S3 config
/// and every named backup target, so they share one purpose-scoped key).
pub(crate) fn seal_secret(plaintext: &[u8]) -> Result<Encrypted, String> {
    encrypt(&master_key(), plaintext)
}

/// Decrypts an at-rest secret back to bytes.
pub(crate) fn open_secret(sealed: &Encrypted) -> Result<Vec<u8>, String> {
    decrypt(
        &master_key(),
        &sealed.wrapped_dek,
        &sealed.dek_nonce,
        &sealed.ciphertext,
        &sealed.nonce,
    )
}

/// Loads the full config with the secret decrypted, for internal use by the
/// backup/restore pipelines and the connectivity check. `Ok(None)` when no
/// S3 destination has been configured yet.
///
/// The destination is the *default* backup target (`is_default = 1`),
/// resolving through the same `backup_s3_targets` table the per-database
/// backups use. If the default target row is missing (pre-0049 installs that
/// never opened the backups page), fall back to the legacy `s3_config` row so
/// nothing breaks mid-upgrade.
pub async fn load(state: &SharedState) -> Result<Option<S3Config>, AppError> {
    let conn = crate::git::repositories::db::get_conn(state).await;
    let Some(row) = sqlx::query(
        "SELECT endpoint_url, bucket, region, access_key_id, wrapped_dek, dek_nonce, ciphertext, nonce,
                schedule_enabled, schedule_interval, schedule_time
         FROM backup_s3_targets WHERE is_default = 1",
    )
    .fetch_optional(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?
    else {
        // Legacy single-row table, kept until the next full migration-lockstep
        // so the read path is one code path for all pre-0049 upgrades.
        let Some(row) = sqlx::query(
            "SELECT endpoint_url, bucket, region, access_key_id, wrapped_dek, dek_nonce, ciphertext, nonce,
                    schedule_enabled, schedule_interval, schedule_time
             FROM s3_config WHERE id = 1",
        )
        .fetch_optional(&conn)
        .await
        .map_err(|e| AppError::DatabaseError(e.to_string()))?
        else {
            return Ok(None);
        };
        return parse_config_row(&row).await;
    };
    parse_config_row(&row).await
}

async fn parse_config_row(row: &sqlx::sqlite::SqliteRow) -> Result<Option<S3Config>, AppError> {
    use sqlx::Row;
    let endpoint_url: String = row
        .try_get(0)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let bucket: String = row
        .try_get(1)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let region: String = row
        .try_get(2)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let access_key_id: String = row
        .try_get(3)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let wrapped_dek: Vec<u8> = row
        .try_get(4)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let dek_nonce: Vec<u8> = row
        .try_get(5)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let ciphertext: Vec<u8> = row
        .try_get(6)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let nonce: Vec<u8> = row
        .try_get(7)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let schedule_enabled: i64 = row
        .try_get(8)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let schedule_interval: Option<String> = row
        .try_get(9)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;
    let schedule_time: Option<String> = row
        .try_get(10)
        .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    let master = master_key();
    let mut secret_bytes = decrypt(&master, &wrapped_dek, &dek_nonce, &ciphertext, &nonce)
        .map_err(AppError::InternalError)?;
    let secret_access_key = String::from_utf8(secret_bytes.clone())
        .map_err(|_| AppError::InternalError("decrypted S3 secret is not utf-8".into()))?;
    secret_bytes.zeroize();

    Ok(Some(S3Config {
        endpoint_url,
        bucket,
        region,
        access_key_id,
        secret_access_key,
        schedule_enabled: schedule_enabled != 0,
        schedule_interval,
        schedule_time,
    }))
}

pub async fn load_dto(state: &SharedState) -> Result<Option<S3ConfigDto>, AppError> {
    Ok(load(state).await?.map(|config| S3ConfigDto {
        endpoint_url: config.endpoint_url,
        bucket: config.bucket,
        region: config.region,
        access_key_id: config.access_key_id,
        has_secret_key: true,
        schedule_enabled: config.schedule_enabled,
        schedule_interval: config.schedule_interval,
        schedule_time: config.schedule_time,
    }))
}

/// Persists the S3 config. Callers must run `test_connection` against the
/// resulting config first — apply externally, persist only on success, same
/// sequencing as the server-domain settings' Caddy sync.
pub async fn save(
    state: &SharedState,
    req: &UpdateS3ConfigRequest,
    secret_access_key: &str,
) -> Result<S3ConfigDto, AppError> {
    let master = master_key();
    let mut secret_bytes = secret_access_key.as_bytes().to_vec();
    let enc = encrypt(&master, &secret_bytes).map_err(AppError::InternalError)?;
    secret_bytes.zeroize();

    let conn = crate::git::repositories::db::get_conn(state).await;
    sqlx::query(
        "INSERT INTO backup_s3_targets \
         (id, name, endpoint_url, bucket, region, access_key_id, wrapped_dek, dek_nonce, ciphertext, nonce, \
          created_at, updated_at, created_by, is_default, schedule_enabled, schedule_interval, schedule_time) \
         VALUES ('system-default', 'Default (system backups)', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, 1, ?11, ?12, ?13) \
         ON CONFLICT(id) DO UPDATE SET \
           name = 'Default (system backups)', \
           endpoint_url = excluded.endpoint_url, \
           bucket = excluded.bucket, \
           region = excluded.region, \
           access_key_id = excluded.access_key_id, \
           wrapped_dek = excluded.wrapped_dek, \
           dek_nonce = excluded.dek_nonce, \
           ciphertext = excluded.ciphertext, \
           nonce = excluded.nonce, \
           updated_at = excluded.updated_at, \
           is_default = 1, \
           schedule_enabled = excluded.schedule_enabled, \
           schedule_interval = excluded.schedule_interval, \
           schedule_time = excluded.schedule_time",
    )
    .bind(req.endpoint_url.clone())
    .bind(req.bucket.clone())
    .bind(req.region.clone())
    .bind(req.access_key_id.clone())
    .bind(enc.wrapped_dek)
    .bind(enc.dek_nonce)
    .bind(enc.ciphertext)
    .bind(enc.nonce)
    .bind(chrono::Utc::now().timestamp_millis())
    .bind(chrono::Utc::now().timestamp_millis())
    .bind(req.schedule_enabled as i64)
    .bind(req.schedule_interval.clone())
    .bind(req.schedule_time.clone())
    .execute(&conn)
    .await
    .map_err(|e| AppError::DatabaseError(e.to_string()))?;

    Ok(S3ConfigDto {
        endpoint_url: req.endpoint_url.clone(),
        bucket: req.bucket.clone(),
        region: req.region.clone(),
        access_key_id: req.access_key_id.clone(),
        has_secret_key: true,
        schedule_enabled: req.schedule_enabled,
        schedule_interval: req.schedule_interval.clone(),
        schedule_time: req.schedule_time.clone(),
    })
}
