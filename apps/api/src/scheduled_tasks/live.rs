//! In-memory push channel for watching a scheduled task execution live.
//!
//! The runner still writes every line to SQLite (the durable record); this only
//! fans the same rows out to open SSE viewers the moment they change, so a viewer
//! never has to poll. A channel exists only while someone is watching: with no
//! subscriber, publishing is a map lookup and nothing else.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde::Serialize;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LiveEvent {
    /// A log row was added or, for the line still being written, replaced.
    Line {
        sequence: i64,
        stream: &'static str,
        message: String,
    },
    /// The execution reached a terminal state; the viewer re-reads the detail.
    Status { status: String },
}

const CHANNEL_CAPACITY: usize = 512;

static CHANNELS: LazyLock<Mutex<HashMap<String, broadcast::Sender<LiveEvent>>>> =
    LazyLock::new(Default::default);

/// Starts watching an execution. Pair with [`release`] once the viewer is gone.
pub fn subscribe(execution_id: &str) -> broadcast::Receiver<LiveEvent> {
    let mut channels = CHANNELS.lock().unwrap_or_else(|error| error.into_inner());
    channels
        .entry(execution_id.to_owned())
        .or_insert_with(|| broadcast::channel(CHANNEL_CAPACITY).0)
        .subscribe()
}

/// Drops the channel once no viewer of the execution is left.
pub fn release(execution_id: &str) {
    let mut channels = CHANNELS.lock().unwrap_or_else(|error| error.into_inner());
    if channels
        .get(execution_id)
        .is_some_and(|sender| sender.receiver_count() == 0)
    {
        channels.remove(execution_id);
    }
}

/// True while at least one viewer watches the execution, so the runner can skip
/// building events nobody would receive.
pub fn is_watched(execution_id: &str) -> bool {
    CHANNELS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(execution_id)
        .is_some_and(|sender| sender.receiver_count() > 0)
}

pub fn publish(execution_id: &str, event: LiveEvent) {
    let channels = CHANNELS.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(sender) = channels.get(execution_id) {
        let _ = sender.send(event);
    }
}

/// Tells viewers the execution ended. The channel itself is dropped by the last
/// viewer's [`release`].
pub fn finished(execution_id: &str, status: &str) {
    publish(
        execution_id,
        LiveEvent::Status {
            status: status.to_owned(),
        },
    );
}

fn is_terminal(status: &str) -> bool {
    !matches!(status, "queued" | "running")
}

fn stream_name(stream: &str) -> &'static str {
    match stream {
        "stdout" => "stdout",
        "stderr" => "stderr",
        _ => "system",
    }
}

async fn read_status(conn: &sqlx::SqlitePool, execution_id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT status FROM scheduled_task_executions WHERE id = ?1")
        .bind(execution_id)
        .fetch_optional(conn)
        .await
        .ok()
        .flatten()
}

/// Current status and every saved log row, as live events.
async fn snapshot(conn: &sqlx::SqlitePool, execution_id: &str) -> (Option<String>, Vec<LiveEvent>) {
    let status = read_status(conn, execution_id).await;
    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT sequence, stream, message FROM scheduled_task_execution_logs WHERE execution_id = ?1 ORDER BY sequence",
    )
    .bind(execution_id)
    .fetch_all(conn)
    .await
    .unwrap_or_default();
    let lines = rows
        .into_iter()
        .map(|(sequence, stream, message)| LiveEvent::Line {
            sequence,
            stream: stream_name(&stream),
            message,
        })
        .collect();
    (status, lines)
}

/// Releases the viewer's channel when its SSE response is dropped.
struct ViewerGuard(String);

impl Drop for ViewerGuard {
    fn drop(&mut self) {
        release(&self.0);
    }
}

/// Everything a viewer needs, in order: the rows saved so far, then each change as it
/// happens, then a final `status` event. Subscribing happens before the snapshot is
/// read, so nothing written in between is missed (a duplicate row is harmless: rows
/// are keyed by sequence). Purely a reader: nothing here can affect the run.
pub fn watch(
    conn: sqlx::SqlitePool,
    execution_id: String,
) -> impl futures_util::Stream<Item = LiveEvent> {
    let receiver = subscribe(&execution_id);
    async_stream::stream! {
        let _guard = ViewerGuard(execution_id.clone());
        let mut receiver = receiver;
        let (status, lines) = snapshot(&conn, &execution_id).await;
        for line in lines {
            yield line;
        }
        if let Some(status) = status.filter(|status| is_terminal(status)) {
            yield LiveEvent::Status { status };
            return;
        }
        // a missed `finished` (e.g. an API restart) must not leave a viewer hanging
        let mut status_check = tokio::time::interval(std::time::Duration::from_secs(3));
        loop {
            tokio::select! {
                event = receiver.recv() => match event {
                    Ok(event @ LiveEvent::Status { .. }) => {
                        yield event;
                        return;
                    }
                    Ok(event) => yield event,
                    // fell behind the channel: resend what is saved and carry on
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        for line in snapshot(&conn, &execution_id).await.1 {
                            yield line;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                },
                _ = status_check.tick() => {
                    if let Some(status) = read_status(&conn, &execution_id).await.filter(|status| is_terminal(status)) {
                        yield LiveEvent::Status { status };
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn viewers_receive_events_and_the_last_one_drops_the_channel() {
        let mut first = subscribe("exec-a");
        let second = subscribe("exec-a");
        assert!(is_watched("exec-a"));
        finished("exec-a", "succeeded");
        assert!(matches!(
            first.recv().await,
            Ok(LiveEvent::Status { status }) if status == "succeeded"
        ));
        drop(first);
        release("exec-a");
        assert!(is_watched("exec-a"));
        drop(second);
        release("exec-a");
        assert!(!is_watched("exec-a"));
        // publishing with nobody watching is a no-op
        publish("exec-a", LiveEvent::Status { status: "x".into() });
    }

    #[tokio::test]
    async fn watch_sends_saved_rows_then_changes_then_the_final_status() {
        use futures_util::StreamExt;
        let conn = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for sql in [
            "CREATE TABLE scheduled_task_executions (id TEXT PRIMARY KEY, status TEXT NOT NULL)",
            "CREATE TABLE scheduled_task_execution_logs (execution_id TEXT, sequence INTEGER, stream TEXT, message TEXT, created_at INTEGER)",
            "INSERT INTO scheduled_task_executions VALUES ('exec-w', 'running')",
            "INSERT INTO scheduled_task_execution_logs VALUES ('exec-w', 0, 'stdout', 'saved', 0)",
        ] {
            sqlx::query(sql).execute(&conn).await.unwrap();
        }
        let mut events = Box::pin(watch(conn, "exec-w".into()));
        assert!(
            matches!(events.next().await, Some(LiveEvent::Line { message, .. }) if message == "saved")
        );
        publish(
            "exec-w",
            LiveEvent::Line {
                sequence: 1,
                stream: "stdout",
                message: "[##] 50%".into(),
            },
        );
        assert!(matches!(
            events.next().await,
            Some(LiveEvent::Line { sequence: 1, .. })
        ));
        finished("exec-w", "succeeded");
        assert!(
            matches!(events.next().await, Some(LiveEvent::Status { status }) if status == "succeeded")
        );
        assert!(events.next().await.is_none());
        drop(events);
        assert!(!is_watched("exec-w"));
    }
}
