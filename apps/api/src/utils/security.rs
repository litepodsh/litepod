use std::env;

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use argon2::{Config, Variant, Version};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use rand::RngExt;

/// 96-bit nonce size for AES-256-GCM.
const NONCE_SIZE: usize = 12;

/// Get the 32-byte encryption key from the ENCRYPTION_KEY env var (64 hex chars).
/// `main.rs` validates this is set and well-formed before the server accepts any
/// requests, so outside of `cargo test` (which never runs `main` and doesn't load
/// `.env`) a missing value means that startup check was bypassed — fail loudly
/// rather than silently falling back to a key derivable from the public source.
/// The `cfg!(test)` branch only exists in test binaries; it is never compiled
/// into the release binary that actually serves traffic.
pub fn get_encryption_key() -> [u8; 32] {
    let env_key = std::env::var("ENCRYPTION_KEY").unwrap_or_else(|_| {
        if cfg!(test) {
            "ab".repeat(32)
        } else {
            panic!("ENCRYPTION_KEY must be set (validated at startup in main.rs)")
        }
    });
    let mut key = [0u8; 32];
    for i in 0..32 {
        let byte_str = &env_key[i * 2..i * 2 + 2];
        key[i] = u8::from_str_radix(byte_str, 16)
            .expect("ENCRYPTION_KEY must be 64 hex characters (validated at startup in main.rs)");
    }
    key
}

/// Encrypt plaintext using AES-256-GCM with a random 12-byte nonce.
/// Returns base64-encoded nonce || ciphertext.
pub fn encrypt_data(plaintext: &str, key: &[u8; 32]) -> Result<String, String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| format!("cipher init: {}", e))?;
    let mut nonce_bytes = [0u8; NONCE_SIZE];
    use rand::Rng;
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| format!("encrypt: {}", e))?;
    let mut combined = nonce_bytes.to_vec();
    combined.extend_from_slice(&ciphertext);
    Ok(BASE64.encode(&combined))
}

/// Decrypt base64-encoded nonce || ciphertext using AES-256-GCM.
pub fn decrypt_data(encrypted: &str, key: &[u8; 32]) -> Result<String, String> {
    let data = BASE64
        .decode(encrypted)
        .map_err(|e| format!("decode: {}", e))?;
    if data.len() < NONCE_SIZE {
        return Err("data too short".to_string());
    }
    let (nonce_bytes, ciphertext) = data.split_at(NONCE_SIZE);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| format!("cipher init: {}", e))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("decrypt: {}", e))?;
    String::from_utf8(plaintext).map_err(|e| format!("utf8: {}", e))
}

// ── Password hashing ──────────────────────────────────────────────────────────

fn test_hash_secret_fallback() -> String {
    if cfg!(test) {
        "test-hash-secret".to_string()
    } else {
        panic!("HASH_SECRET must be set (validated at startup in main.rs)")
    }
}

pub fn hash_password(pass: String) -> String {
    let hash_secret = env::var("HASH_SECRET").unwrap_or_else(|_| test_hash_secret_fallback());
    let salt: [u8; 32] = rand::rng().random();
    let config = Config {
        variant: Variant::Argon2id,
        version: Version::Version13,
        mem_cost: 65536,
        time_cost: 4,
        lanes: 16,
        secret: hash_secret.as_bytes(),
        ad: &[],
        hash_length: 32,
        thread_mode: argon2::ThreadMode::Parallel,
    };
    argon2::hash_encoded(pass.as_bytes(), &salt, &config).expect("password hashing failed")
}

pub fn verify_password(hashed_pass: &str, password: &[u8]) -> bool {
    let hash_secret = env::var("HASH_SECRET").unwrap_or_else(|_| test_hash_secret_fallback());
    argon2::verify_encoded_ext(hashed_pass, password, hash_secret.as_bytes(), &[]).unwrap_or(false)
}

/// An Argon2id hash of a fixed string, used to spend the same time verifying a
/// password for an address that has no account as for one that does.
///
/// Without it, "no such user" returns in microseconds while a real account costs
/// a full Argon2id verification (`mem_cost 65536`, `time_cost 4`), and the gap is
/// wide enough to enumerate accounts by timing alone. Computed once per process.
static DUMMY_PASSWORD_HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Burns one password verification against a throwaway hash. Call on the paths
/// where no real hash is available, so every branch costs the same.
pub fn verify_password_dummy(password: &[u8]) {
    let hash = DUMMY_PASSWORD_HASH
        .get_or_init(|| hash_password("litepod-timing-equalizer-not-a-real-password".to_string()));
    let _ = verify_password(hash, password);
}

/// 256 bits from the OS CSPRNG, hex-encoded.
///
/// Session tokens used to be UUIDv7, which spends 48 of its bits on a readable
/// creation timestamp and leaves roughly 74 random ones — fine for a database
/// key, not for a bearer credential. This is the value to reach for whenever a
/// secret has to be unguessable.
pub fn generate_token() -> String {
    use rand::Rng;
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// SHA-256 of `value`, hex-encoded.
///
/// The storage form for anything that only ever needs to be *checked*, never
/// read back: session tokens, recovery codes, reset tokens. Already the pattern
/// used by `cqrs::commands::invitation` and `email_verification`.
pub fn digest_token(value: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(value.as_bytes()))
}
