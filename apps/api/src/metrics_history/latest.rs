//! Last `podman stats` snapshot per server, kept by the sampler so the dashboard
//! reads it instead of running Podman (over SSH for remote servers) on every poll.

use std::{
    collections::HashMap,
    sync::{LazyLock, RwLock},
};

use crate::podman::podman_metrics::ContainerMetricsSnapshot;

static LATEST: LazyLock<RwLock<HashMap<String, ContainerMetricsSnapshot>>> =
    LazyLock::new(Default::default);

pub(super) fn store(server_id: &str, snapshot: &ContainerMetricsSnapshot) {
    LATEST
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(server_id.to_owned(), snapshot.clone());
}

/// The server's last snapshot while it can stand in for a live read. Two missed
/// ticks and it counts as gone, so a server the sampler stopped reaching never
/// shows numbers from long ago.
pub fn fresh(server_id: &str) -> Option<ContainerMetricsSnapshot> {
    let max_age = super::sample_interval() * 2 + super::SAMPLE_TIMEOUT;
    let latest = LATEST
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let snapshot = latest.get(server_id)?;
    let age = (chrono::Utc::now() - snapshot.observed_at)
        .to_std()
        .unwrap_or_default();
    (age <= max_age).then(|| snapshot.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshot_is_fresh_until_two_ticks_pass() {
        let now = chrono::Utc::now();
        store(
            "srv-fresh",
            &ContainerMetricsSnapshot {
                observed_at: now,
                metrics: vec![],
            },
        );
        assert!(fresh("srv-fresh").is_some());

        let old = now - chrono::Duration::hours(1);
        store(
            "srv-stale",
            &ContainerMetricsSnapshot {
                observed_at: old,
                metrics: vec![],
            },
        );
        assert!(fresh("srv-stale").is_none());
        assert!(fresh("srv-unknown").is_none());
    }
}
