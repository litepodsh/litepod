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

/// Most keys walked for the per-prefix breakdown; keeps the debug read cheap on a busy store.
const DEBUG_SCAN_LIMIT: usize = 10_000;

/// Read-only report for the debug page: server counters from `INFO` plus key counts grouped by
/// prefix. Never returns key values.
#[derive(Debug, Serialize)]
pub struct DebugStats {
    pub version: Option<String>,
    pub uptime_secs: Option<u64>,
    pub used_memory_bytes: Option<u64>,
    pub peak_memory_bytes: Option<u64>,
    /// 0 means no limit was configured.
    pub max_memory_bytes: Option<u64>,
    pub connected_clients: Option<u64>,
    pub total_commands: Option<u64>,
    pub keyspace_hits: Option<u64>,
    pub keyspace_misses: Option<u64>,
    pub expired_keys: Option<u64>,
    pub evicted_keys: Option<u64>,
    pub key_count: u64,
    /// Second segment of `litepod:<group>:...` keys (whole key otherwise), largest first.
    pub prefixes: Vec<KeyPrefix>,
    /// True when the scan stopped at `DEBUG_SCAN_LIMIT` keys, so `prefixes` is partial.
    pub prefixes_truncated: bool,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct KeyPrefix {
    pub name: String,
    pub keys: u64,
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

    pub async fn debug_stats(&self) -> Result<DebugStats, String> {
        let mut connection = self.connection().await?;
        let info: String = redis::cmd("INFO")
            .query_async(&mut connection)
            .await
            .map_err(|error| error.to_string())?;
        let key_count: u64 = redis::cmd("DBSIZE")
            .query_async(&mut connection)
            .await
            .map_err(|error| error.to_string())?;

        let mut counts = std::collections::HashMap::<String, u64>::new();
        let mut scanned = 0usize;
        let mut cursor = 0u64;
        loop {
            let (next, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("COUNT")
                .arg(500)
                .query_async(&mut connection)
                .await
                .map_err(|error| error.to_string())?;
            scanned += keys.len();
            for key in keys {
                *counts.entry(key_prefix(&key)).or_default() += 1;
            }
            cursor = next;
            if cursor == 0 || scanned >= DEBUG_SCAN_LIMIT {
                break;
            }
        }
        let mut prefixes: Vec<KeyPrefix> = counts
            .into_iter()
            .map(|(name, keys)| KeyPrefix { name, keys })
            .collect();
        prefixes.sort_by(|a, b| b.keys.cmp(&a.keys).then_with(|| a.name.cmp(&b.name)));

        let field = |name: &str| info_field(&info, name);
        let number = |name: &str| field(name).and_then(|value| value.parse::<u64>().ok());
        Ok(DebugStats {
            version: field("dragonfly_version").or_else(|| field("redis_version")),
            uptime_secs: number("uptime_in_seconds"),
            used_memory_bytes: number("used_memory"),
            peak_memory_bytes: number("used_memory_peak"),
            max_memory_bytes: number("maxmemory"),
            connected_clients: number("connected_clients"),
            total_commands: number("total_commands_processed"),
            keyspace_hits: number("keyspace_hits"),
            keyspace_misses: number("keyspace_misses"),
            expired_keys: number("expired_keys"),
            evicted_keys: number("evicted_keys"),
            key_count,
            prefixes,
            prefixes_truncated: cursor != 0,
        })
    }
}

/// `INFO` is `name:value` lines grouped under `# Section` headers.
fn info_field(info: &str, name: &str) -> Option<String> {
    info.lines().find_map(|line| {
        let (key, value) = line.trim_end().split_once(':')?;
        (key == name).then(|| value.to_string())
    })
}

/// Litepod keys look like `litepod:<group>:<id>...`; anything else is reported whole.
fn key_prefix(key: &str) -> String {
    let mut parts = key.splitn(3, ':');
    match (parts.next(), parts.next()) {
        (Some("litepod"), Some(group)) if !group.is_empty() => group.to_string(),
        _ => key.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_field_reads_named_value() {
        let info = "# Server\r\nredis_version:7.4.0\r\ndragonfly_version:df-v1.39.0\r\n# Memory\r\nused_memory:1024\r\n";
        assert_eq!(
            info_field(info, "dragonfly_version").as_deref(),
            Some("df-v1.39.0")
        );
        assert_eq!(info_field(info, "used_memory").as_deref(), Some("1024"));
        assert_eq!(info_field(info, "used_memory_peak"), None);
    }

    #[test]
    fn key_prefix_groups_litepod_keys() {
        assert_eq!(
            key_prefix("litepod:rate-limit:global:1.2.3.4"),
            "rate-limit"
        );
        assert_eq!(key_prefix("litepod:login:1.2.3.4:failures"), "login");
        assert_eq!(key_prefix("other"), "other");
        assert_eq!(key_prefix("litepod:"), "litepod:");
    }
}
