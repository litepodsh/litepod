use std::time::{Duration, SystemTime, UNIX_EPOCH};

use redis::{AsyncCommands, Script};
use serde::{Serialize, de::DeserializeOwned};

/// Dragonfly/Redis lives in its own container and can be briefly unreachable (restart, network
/// hiccup) independent of the API. `get_multiplexed_async_connection` has no built-in timeout, so
/// without one a hiccup hangs on OS-level TCP retry (seconds, not milliseconds) instead of falling
/// back to the local rate limiter like a normal connection error does. Same-podman-network hops
/// take low single-digit ms; 300ms is generous headroom.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);

#[derive(Clone, Default)]
pub struct EphemeralStore {
    client: Option<redis::Client>,
}

#[derive(Debug, Clone, Copy)]
pub struct TokenBucketResult {
    pub allowed: bool,
    pub remaining: u32,
    pub retry_after_secs: u64,
}

impl EphemeralStore {
    pub fn from_env() -> Self {
        let url = std::env::var("DRAGONFLY_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "redis://127.0.0.1:6379".into());
        Self {
            client: redis::Client::open(url).ok(),
        }
    }

    async fn connection(&self) -> Result<redis::aio::MultiplexedConnection, String> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| "Dragonfly is disabled".to_string())?;
        tokio::time::timeout(CONNECT_TIMEOUT, client.get_multiplexed_async_connection())
            .await
            .map_err(|_| "Dragonfly connection timed out".to_string())?
            .map_err(|error| error.to_string())
    }

    pub async fn get_json<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, String> {
        let mut connection = self.connection().await?;
        let raw: Option<String> = connection
            .get(key)
            .await
            .map_err(|error| error.to_string())?;
        raw.map(|value| serde_json::from_str(&value).map_err(|error| error.to_string()))
            .transpose()
    }

    /// Reads JSON and refreshes its expiry in one Redis operation. Missing keys stay missing.
    pub async fn get_json_refresh<T: DeserializeOwned>(
        &self,
        key: &str,
        ttl_secs: u64,
    ) -> Result<Option<T>, String> {
        let mut connection = self.connection().await?;
        let raw: Option<String> = Script::new(
            "local value = redis.call('GET', KEYS[1]); \
             if value then redis.call('EXPIRE', KEYS[1], ARGV[1]); end; \
             return value",
        )
        .key(key)
        .arg(ttl_secs)
        .invoke_async(&mut connection)
        .await
        .map_err(|error| error.to_string())?;
        raw.map(|value| serde_json::from_str(&value).map_err(|error| error.to_string()))
            .transpose()
    }

    pub async fn set_json<T: Serialize>(
        &self,
        key: &str,
        value: &T,
        ttl_secs: u64,
    ) -> Result<(), String> {
        let raw = serde_json::to_string(value).map_err(|error| error.to_string())?;
        let mut connection = self.connection().await?;
        connection
            .set_ex::<_, _, ()>(key, raw, ttl_secs)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn delete(&self, key: &str) -> Result<(), String> {
        let mut connection = self.connection().await?;
        connection
            .del::<_, ()>(key)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn take_json<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, String> {
        let mut connection = self.connection().await?;
        let raw: Option<String> = redis::cmd("GETDEL")
            .arg(key)
            .query_async(&mut connection)
            .await
            .map_err(|error| error.to_string())?;
        raw.map(|value| serde_json::from_str(&value).map_err(|error| error.to_string()))
            .transpose()
    }

    pub async fn set_json_if_absent<T: Serialize>(
        &self,
        key: &str,
        value: &T,
        ttl_secs: u64,
    ) -> Result<bool, String> {
        let raw = serde_json::to_string(value).map_err(|error| error.to_string())?;
        let mut connection = self.connection().await?;
        let set: Option<String> = redis::cmd("SET")
            .arg(key)
            .arg(raw)
            .arg("NX")
            .arg("EX")
            .arg(ttl_secs)
            .query_async(&mut connection)
            .await
            .map_err(|error| error.to_string())?;
        Ok(set.is_some())
    }

    pub async fn remaining_ttl(&self, key: &str) -> Result<Option<u64>, String> {
        let mut connection = self.connection().await?;
        let seconds: i64 = redis::cmd("TTL")
            .arg(key)
            .query_async(&mut connection)
            .await
            .map_err(|error| error.to_string())?;
        Ok((seconds > 0).then_some(seconds as u64))
    }

    /// Reads a plain integer counter (e.g. the `:failures` key `record_login_failure` writes
    /// via `INCR`), not a JSON value. `None` when the key is absent (no failures recorded, or
    /// it already expired).
    pub async fn get_counter(&self, key: &str) -> Result<Option<u32>, String> {
        let mut connection = self.connection().await?;
        let raw: Option<i64> = connection
            .get(key)
            .await
            .map_err(|error| error.to_string())?;
        Ok(raw.map(|value| value.max(0) as u32))
    }

    pub async fn record_login_failure(
        &self,
        key: &str,
        max_attempts: u32,
        ban_secs: u64,
    ) -> Result<Option<u64>, String> {
        let script = Script::new(
            "if redis.call('EXISTS', KEYS[2]) == 1 then return redis.call('TTL', KEYS[2]); end; \
             local count = redis.call('INCR', KEYS[1]); \
             if count == 1 then redis.call('EXPIRE', KEYS[1], ARGV[2]); end; \
             if count >= tonumber(ARGV[1]) then \
               redis.call('SET', KEYS[2], '1', 'EX', ARGV[2]); redis.call('DEL', KEYS[1]); return tonumber(ARGV[2]); \
             end; return 0",
        );
        let mut connection = self.connection().await?;
        let seconds: i64 = script
            .key(format!("{key}:failures"))
            .key(format!("{key}:ban"))
            .arg(max_attempts.max(1))
            .arg(ban_secs.max(1))
            .invoke_async(&mut connection)
            .await
            .map_err(|error| error.to_string())?;
        Ok((seconds > 0).then_some(seconds as u64))
    }

    pub async fn fixed_window_increment(&self, key: &str, ttl_secs: u64) -> Result<u64, String> {
        let script = Script::new(
            "local count = redis.call('INCR', KEYS[1]); \
             if count == 1 then redis.call('EXPIRE', KEYS[1], ARGV[1]); end; \
             return count",
        );
        let mut connection = self.connection().await?;
        script
            .key(key)
            .arg(ttl_secs)
            .invoke_async(&mut connection)
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn token_bucket(
        &self,
        key: &str,
        per_second: u32,
        burst: u32,
    ) -> Result<TokenBucketResult, String> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_millis() as i64;
        let script = Script::new(
            "local now = tonumber(ARGV[1]); \
             local rate = tonumber(ARGV[2]); \
             local burst = tonumber(ARGV[3]); \
             local values = redis.call('HMGET', KEYS[1], 'tokens', 'updated'); \
             local tokens = tonumber(values[1]) or burst; \
             local updated = tonumber(values[2]) or now; \
             tokens = math.min(burst, tokens + math.max(0, now - updated) * rate / 1000); \
             local allowed = tokens >= 1; \
             if allowed then tokens = tokens - 1; end; \
             redis.call('HMSET', KEYS[1], 'tokens', tokens, 'updated', now); \
             redis.call('PEXPIRE', KEYS[1], math.ceil((burst / rate) * 2000)); \
             local retry = 0; \
             if not allowed then retry = math.max(1, math.ceil((1 - tokens) / rate)); end; \
             return {allowed and 1 or 0, math.floor(tokens), retry}",
        );
        let mut connection = self.connection().await?;
        let values: Vec<i64> = script
            .key(key)
            .arg(now_ms)
            .arg(per_second.max(1))
            .arg(burst.max(per_second.max(1)))
            .invoke_async(&mut connection)
            .await
            .map_err(|error| error.to_string())?;
        if values.len() != 3 {
            return Err("Dragonfly token bucket returned an invalid response".into());
        }
        Ok(TokenBucketResult {
            allowed: values[0] == 1,
            remaining: values[1].max(0) as u32,
            retry_after_secs: values[2].max(0) as u64,
        })
    }
}
