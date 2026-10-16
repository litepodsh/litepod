use chrono::Utc;

pub(super) fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub(super) const REMOVE_VOLUMES_WARNING: &str =
    "WARNING: Removing Compose runtime and Compose-managed volumes";

pub(super) fn tail(output: &[u8]) -> String {
    String::from_utf8_lossy(output)
        .lines()
        .rev()
        .take(40)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}
