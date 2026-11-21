//! Moves backup bytes between a Podman command and S3 without holding them in
//! memory: database dumps, volume exports and their restores all pass through
//! here as streams, so their size is bounded by disk and S3, not the API's RAM.

use std::time::Duration;

use s3::Bucket;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::task::JoinHandle;

use crate::dtos::response::AppError;
use crate::servers::ssh::{SshStream, clamp_tail};

/// Reads a pipe to the end in the background, so a command never blocks on a
/// full stderr/stdout nobody reads. Yields the text for error messages.
fn drain<R: AsyncRead + Unpin + Send + 'static>(pipe: Option<R>) -> JoinHandle<String> {
    tokio::spawn(async move {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = tokio::io::AsyncReadExt::read_to_end(&mut pipe, &mut bytes).await;
        }
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

/// Turns the command's exit into an error carrying its stderr tail.
fn exit_error(
    exit: Result<std::process::ExitStatus, AppError>,
    stderr: &str,
    what: &str,
) -> Option<AppError> {
    match exit {
        Err(error) => Some(error),
        Ok(status) if !status.success() => Some(AppError::InternalError(format!(
            "{what} failed: {}",
            clamp_tail(stderr, 500)
        ))),
        Ok(_) => None,
    }
}

fn timed_out(what: &str) -> AppError {
    AppError::InternalError(format!("{what} timed out"))
}

/// Uploads `reader` as `key`. rust-s3 runs without `fail-on-err`, so the
/// status must be checked here; an empty upload is never a usable backup.
pub async fn put_reader<R: AsyncRead + Unpin + ?Sized>(
    bucket: &Bucket,
    key: &str,
    reader: &mut R,
) -> Result<i64, AppError> {
    let response = bucket
        .put_object_stream(reader, key)
        .await
        .map_err(|error| AppError::InternalError(format!("S3 upload failed: {error}")))?;
    if response.status_code() >= 300 {
        return Err(AppError::InternalError(format!(
            "S3 upload failed with status {}",
            response.status_code()
        )));
    }
    if response.uploaded_bytes() == 0 {
        return Err(AppError::InternalError("The backup was empty".into()));
    }
    Ok(response.uploaded_bytes() as i64)
}

/// Opens `key` for reading. The status is checked before any byte is handed
/// on, so an S3 error page never reaches a restore as if it were the data.
pub async fn object_reader(
    bucket: &Bucket,
    key: &str,
) -> Result<impl AsyncRead + Unpin + Send + use<>, AppError> {
    let response = bucket
        .get_object_stream(key)
        .await
        .map_err(|error| AppError::InternalError(format!("S3 download failed: {error}")))?;
    if response.status_code >= 300 {
        return Err(AppError::InternalError(format!(
            "S3 download failed with status {}",
            response.status_code
        )));
    }
    Ok(tokio_util::io::StreamReader::new(
        tokio_stream::StreamExt::map(response.bytes, |chunk| chunk.map_err(std::io::Error::other)),
    ))
}

/// Uploads the command's stdout as `key`. Any failure (command, upload,
/// timeout) deletes the object so partial output never looks like a backup.
pub async fn upload_stdout(
    bucket: &Bucket,
    key: &str,
    mut command: SshStream,
    budget: Duration,
    what: &str,
) -> Result<i64, AppError> {
    let mut stdout = command
        .take_stdout()
        .ok_or_else(|| AppError::InternalError(format!("{what} produced no output stream")))?;
    let stderr = drain(command.take_stderr());
    let outcome = tokio::time::timeout(budget, async {
        let uploaded = put_reader(bucket, key, &mut stdout).await;
        if uploaded.is_err() {
            // Nobody reads stdout any more; stop the command instead of waiting on it.
            command.kill().await;
        }
        (uploaded, command.wait().await)
    })
    .await;
    // Ends a timed-out command so its pipes close and `stderr` can finish.
    drop(command);
    let stderr = stderr.await.unwrap_or_default();
    let result = match outcome {
        Err(_) => Err(timed_out(what)),
        Ok((uploaded, exit)) => match exit_error(exit, &stderr, what) {
            Some(error) => Err(error),
            None => uploaded,
        },
    };
    if result.is_err() {
        let _ = bucket.delete_object(key).await;
    }
    result
}

/// Copies the command's stdout into `writer` (a spool file) and waits for a
/// clean exit. Returns the byte count.
pub async fn stdout_to<W: AsyncWrite + Unpin + ?Sized>(
    mut command: SshStream,
    writer: &mut W,
    budget: Duration,
    what: &str,
) -> Result<u64, AppError> {
    let mut stdout = command
        .take_stdout()
        .ok_or_else(|| AppError::InternalError(format!("{what} produced no output stream")))?;
    let stderr = drain(command.take_stderr());
    let outcome = tokio::time::timeout(budget, async {
        let copied = tokio::io::copy(&mut stdout, writer).await;
        if copied.is_err() {
            command.kill().await;
        }
        (copied, command.wait().await)
    })
    .await;
    drop(command);
    let stderr = stderr.await.unwrap_or_default();
    let (copied, exit) = outcome.map_err(|_| timed_out(what))?;
    if let Some(error) = exit_error(exit, &stderr, what) {
        return Err(error);
    }
    copied.map_err(|error| AppError::InternalError(format!("{what} failed: {error}")))
}

/// Writes `reader` to the command's stdin, closes it, and waits for a clean
/// exit. A command that stops early is reported by its own stderr, not by the
/// broken pipe it leaves behind.
pub async fn feed_stdin<R: AsyncRead + Unpin + ?Sized>(
    mut command: SshStream,
    reader: &mut R,
    budget: Duration,
    what: &str,
) -> Result<(), AppError> {
    let mut stdin = command
        .take_stdin()
        .ok_or_else(|| AppError::InternalError(format!("{what} has no input stream")))?;
    let stdout = drain(command.take_stdout());
    let stderr = drain(command.take_stderr());
    let outcome = tokio::time::timeout(budget, async {
        let copied = tokio::io::copy(reader, &mut stdin).await;
        drop(stdin); // EOF: the command finishes reading.
        (copied, command.wait().await)
    })
    .await;
    drop(command);
    let _ = stdout.await;
    let stderr = stderr.await.unwrap_or_default();
    let (copied, exit) = outcome.map_err(|_| timed_out(what))?;
    if let Some(error) = exit_error(exit, &stderr, what) {
        return Err(error);
    }
    copied
        .map(|_| ())
        .map_err(|error| AppError::InternalError(format!("{what} failed: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> SshStream {
        let child = tokio::process::Command::new("sh")
            .args(["-c", script])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        SshStream::local(child)
    }

    const BUDGET: Duration = Duration::from_secs(10);

    #[tokio::test]
    async fn feed_stdin_delivers_every_byte() {
        let data: Vec<u8> = (0..=255u8).cycle().take(1 << 20).collect();
        let check = format!("test \"$(wc -c)\" -eq {}", data.len());
        feed_stdin(sh(&check), &mut data.as_slice(), BUDGET, "Import")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn feed_stdin_reports_the_command_stderr_not_the_broken_pipe() {
        let data = vec![0u8; 1 << 20];
        let error = feed_stdin(
            sh("echo bad archive >&2; exit 3"),
            &mut data.as_slice(),
            BUDGET,
            "Import",
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("bad archive"), "{error}");
    }

    #[tokio::test]
    async fn stdout_to_copies_and_fails_on_exit_status() {
        let mut out = Vec::new();
        let copied = stdout_to(sh("printf hello"), &mut out, BUDGET, "Export")
            .await
            .unwrap();
        assert_eq!((copied, out.as_slice()), (5, b"hello".as_slice()));
        let mut out = Vec::new();
        let error = stdout_to(
            sh("printf partial; echo boom >&2; exit 1"),
            &mut out,
            BUDGET,
            "Export",
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("boom"), "{error}");
    }

    #[tokio::test]
    async fn a_hung_command_times_out_instead_of_hanging() {
        let mut out = Vec::new();
        let error = stdout_to(
            sh("sleep 30"),
            &mut out,
            Duration::from_millis(200),
            "Export",
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.contains("timed out"), "{error}");
    }
}
