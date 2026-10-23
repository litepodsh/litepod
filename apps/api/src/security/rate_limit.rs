use std::collections::HashMap;
use std::net::IpAddr;
use std::num::NonZeroU32;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use governor::{
    Quota, RateLimiter, clock::DefaultClock, middleware::StateInformationMiddleware,
    state::keyed::DefaultKeyedStateStore,
};
use sqlx::Row;

/// `StateInformationMiddleware` makes `check_key` return burst/quota info on both the
/// allowed and rejected paths, so the middleware can set `X-RateLimit-*`/`Retry-After` headers.
pub type KeyedLimiter =
    RateLimiter<IpAddr, DefaultKeyedStateStore<IpAddr>, DefaultClock, StateInformationMiddleware>;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct RateLimitConfig {
    pub enabled: bool,
    pub global_per_second: u32,
    pub global_burst: u32,
    pub auth_per_second: u32,
    pub auth_burst: u32,
    pub auth_max_attempts: u32,
    pub auth_ban_minutes: i64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            global_per_second: 40,
            global_burst: 80,
            auth_per_second: 5,
            auth_burst: 10,
            auth_max_attempts: 5,
            auth_ban_minutes: 15,
        }
    }
}

#[derive(Default)]
pub struct LoginAttemptEntry {
    pub failed_count: u32,
    pub banned_until: Option<DateTime<Utc>>,
}

pub struct RateLimiterState {
    pub config: RateLimitConfig,
    pub global: Arc<KeyedLimiter>,
    pub auth: Arc<KeyedLimiter>,
    pub login_attempts: HashMap<IpAddr, LoginAttemptEntry>,
}

fn build_limiter(per_second: u32, burst: u32) -> KeyedLimiter {
    let per_second = NonZeroU32::new(per_second.max(1)).unwrap();
    let burst = NonZeroU32::new(burst.max(per_second.get())).unwrap();
    RateLimiter::keyed(Quota::per_second(per_second).allow_burst(burst))
        .with_middleware::<StateInformationMiddleware>()
}

impl RateLimiterState {
    pub fn from_config(config: RateLimitConfig) -> Self {
        let global = Arc::new(build_limiter(config.global_per_second, config.global_burst));
        let auth = Arc::new(build_limiter(config.auth_per_second, config.auth_burst));
        Self {
            config,
            global,
            auth,
            login_attempts: HashMap::new(),
        }
    }

    /// Rebuilds both limiters against the new quotas and drops the old ones — in-flight
    /// requests still holding an `Arc` to the previous limiter finish against it, new
    /// requests immediately see the new quota. Login ban counters/bans are left untouched.
    pub fn apply_config(&mut self, config: RateLimitConfig) {
        self.global = Arc::new(build_limiter(config.global_per_second, config.global_burst));
        self.auth = Arc::new(build_limiter(config.auth_per_second, config.auth_burst));
        self.config = config;
    }
}

/// Reads the singleton settings row directly off the pool so this can run during
/// boot, before `SharedState` exists. Falls back to defaults if the row/table isn't
/// there yet (e.g. migrations haven't created it on a very first boot ordering edge case).
pub async fn load_config(pool: &sqlx::SqlitePool) -> RateLimitConfig {
    let Ok(Some(row)) = sqlx::query(
        "SELECT enabled, global_per_second, global_burst, auth_per_second, auth_burst, auth_max_attempts, auth_ban_minutes FROM rate_limit_settings WHERE id = 1",
    )
    .fetch_optional(pool)
    .await
    else {
        return RateLimitConfig::default();
    };
    RateLimitConfig {
        enabled: row.try_get::<i64, _>(0).unwrap_or(1) != 0,
        global_per_second: row.try_get::<i64, _>(1).unwrap_or(20).max(1) as u32,
        global_burst: row.try_get::<i64, _>(2).unwrap_or(40).max(1) as u32,
        auth_per_second: row.try_get::<i64, _>(3).unwrap_or(5).max(1) as u32,
        auth_burst: row.try_get::<i64, _>(4).unwrap_or(10).max(1) as u32,
        auth_max_attempts: row.try_get::<i64, _>(5).unwrap_or(5).max(1) as u32,
        auth_ban_minutes: row.try_get::<i64, _>(6).unwrap_or(15).max(1),
    }
}
