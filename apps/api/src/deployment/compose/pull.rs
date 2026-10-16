use std::{ffi::OsString, time::Duration};

use tokio::io::{AsyncBufReadExt, BufReader};

use crate::{deployment::history, servers::runtime::ServerRuntime, state::state::SharedState};

/// One image-bearing service discovered in a Compose document. Services that
/// declare only a `build:` context (no `image:`) are returned with `image =
/// None` so the deploy path can skip them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeServiceImage {
    pub name: String,
    pub image: Option<String>,
}

/// Walks a Compose document and returns every service that pins a registry
/// image. Malformed YAML surfaces the parse error verbatim so the existing
/// validation step can echo it back to the operator.
pub fn extract_service_images(content: &str) -> Result<Vec<ComposeServiceImage>, String> {
    let document: serde_yaml::Value =
        serde_yaml::from_str(content).map_err(|error| format!("Invalid Compose file: {error}"))?;
    let services = document
        .get("services")
        .and_then(serde_yaml::Value::as_mapping)
        .ok_or_else(|| "Invalid Compose file: expected a services mapping".to_string())?;
    let mut out = Vec::with_capacity(services.len());
    for (name, config) in services {
        let Some(name) = name.as_str() else { continue };
        let image = config
            .get("image")
            .and_then(serde_yaml::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        out.push(ComposeServiceImage {
            name: name.to_owned(),
            image,
        });
    }
    Ok(out)
}

fn podman_args<const N: usize>(args: [&str; N]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

async fn log_run_line(state: &SharedState, application_id: &str, run_id: Option<&str>, line: &str) {
    tracing::info!(application_id, "[compose-pull] {line}");
    if let Some(run_id) = run_id {
        let _ = history::append(state, run_id, "info", line).await;
    }
}

/// True when podman's inspect stderr means "this image is simply not here
/// yet". The wording drifts between podman versions (`image not known`,
/// `failed to find image`, older `no such image`) and the exit code is not a
/// reliable discriminator, so match on the message instead.
fn is_missing_image_error(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("image not known")
        || stderr.contains("failed to find image")
        || stderr.contains("no such image")
}

/// Returns the local digest of `image` on `runtime`, or `None` if the image
/// is not present there yet. `--format '{{.Id}}'` makes podman print a bare
/// digest string, so no JSON parsing is involved; a missing image is
/// reported as a failed exit with a "not known" message on stderr and we
/// treat that as `None`.
pub async fn podman_image_digest(
    runtime: &ServerRuntime,
    image: &str,
) -> Result<Option<String>, String> {
    let args = podman_args(["image", "inspect", "--format", "{{.Id}}", image]);
    match runtime.podman(&args).await {
        Ok(output) => {
            let digest = output.stdout.trim().to_owned();
            Ok(if digest.is_empty() {
                None
            } else {
                Some(digest)
            })
        }
        Err(error) => {
            let message = error.to_string();
            if is_missing_image_error(&message) {
                Ok(None)
            } else {
                Err(format!("Could not inspect image {image}: {message}"))
            }
        }
    }
}

/// Streams `podman pull <image>` on `runtime` (local or, for a remote
/// deployment, over SSH on the actual target server) to the run log and
/// resolves to the fresh digest on success. The error path returns the tail
/// of stderr so the queue stream surfaces the registry's own failure reason.
pub async fn podman_pull(
    runtime: &ServerRuntime,
    state: &SharedState,
    application_id: &str,
    run_id: Option<&str>,
    image: &str,
) -> Result<String, String> {
    log_run_line(
        state,
        application_id,
        run_id,
        &format!("Pulling image {image}"),
    )
    .await;
    let args = podman_args(["pull", image]);
    let mut stream = runtime
        .podman_stream(&args)
        .await
        .map_err(|error| format!("Could not start podman pull {image}: {error}"))?;
    let stdout = stream.take_stdout();
    let stderr = stream.take_stderr();
    let stream_state = state.clone();
    let stream_app = application_id.to_owned();
    let stream_run = run_id.map(str::to_owned);
    let stderr_tail: Vec<String> = match (stdout, stderr) {
        (Some(stdout), Some(stderr)) => {
            let stdout_state = stream_state.clone();
            let stdout_app = stream_app.clone();
            let stdout_run = stream_run.clone();
            let stdout_handle = tokio::spawn(async move {
                stream_pull_output(stdout, stdout_state, stdout_app, stdout_run.as_deref()).await
            });
            let stderr_handle = tokio::spawn(async move {
                stream_pull_output(stderr, stream_state, stream_app, stream_run.as_deref()).await
            });
            let (stdout_result, stderr_result) = tokio::join!(stdout_handle, stderr_handle);
            let _ = stdout_result;
            stderr_result.unwrap_or_default()
        }
        _ => Vec::new(),
    };

    // A stalled registry connection (rate limiting, a silently-dropped
    // connection) must fail loudly instead of hanging the whole deployment
    // forever - same 30-minute budget + kill the compose up/down steps use.
    let status = match tokio::time::timeout(Duration::from_secs(30 * 60), stream.wait()).await {
        Ok(result) => {
            Some(result.map_err(|error| format!("Could not finish podman pull {image}: {error}"))?)
        }
        Err(_) => {
            stream.kill().await;
            let _ = stream.wait().await;
            None
        }
    };
    let Some(status) = status else {
        return Err(format!("podman pull {image} timed out after 30 minutes"));
    };
    if !status.success() {
        let tail_text = stderr_tail.join("\n");
        let detail = if tail_text.is_empty() {
            format!("exit code {}", status.code().unwrap_or(-1))
        } else {
            tail_text
        };
        return Err(format!("podman pull {image} failed: {detail}"));
    }
    podman_image_digest(runtime, image).await?.ok_or_else(|| {
        format!("podman pull {image} reported success but the image is missing locally")
    })
}

async fn stream_pull_output<R>(
    reader: R,
    state: SharedState,
    application_id: String,
    run_id: Option<&str>,
) -> Vec<String>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let mut tail = Vec::new();
    loop {
        line.clear();
        let read = match reader.read_line(&mut line).await {
            Ok(value) => value,
            Err(_) => return tail,
        };
        if read == 0 {
            return tail;
        }
        let message = line.trim_end_matches(['\r', '\n']).to_owned();
        if message.is_empty() {
            continue;
        }
        log_run_line(&state, &application_id, run_id, &message).await;
        tail.push(message);
        if tail.len() > 40 {
            tail.remove(0);
        }
    }
}

/// Pulls every registry image declared in `content` via `podman pull` on
/// `runtime`, then returns the new digests paired with what was there
/// before. Pull errors propagate so a registry outage fails the deploy
/// instead of silently leaving the cached image in place.
pub async fn pull_compose_images(
    runtime: &ServerRuntime,
    state: &SharedState,
    application_id: &str,
    run_id: Option<&str>,
    content: &str,
) -> Result<Vec<PulledImage>, String> {
    let services = extract_service_images(content)?;
    let mut pulled = Vec::with_capacity(services.len());
    for service in services {
        let Some(image) = service.image else {
            continue;
        };
        let before = podman_image_digest(runtime, &image).await?;
        let after = podman_pull(runtime, state, application_id, run_id, &image).await?;
        let changed = before.as_deref() != Some(after.as_str());
        log_run_line(
            state,
            application_id,
            run_id,
            &format!(
                "Image {image} digest {}{}",
                short(&after),
                if changed {
                    " (updated)"
                } else {
                    " (unchanged)"
                }
            ),
        )
        .await;
        pulled.push(PulledImage {
            service: service.name,
            image,
            before,
            after,
        });
    }
    Ok(pulled)
}

/// Result of pulling one image during a deploy. Surfaced in the run log so the
/// operator can see exactly which digest landed in the container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PulledImage {
    pub service: String,
    pub image: String,
    pub before: Option<String>,
    pub after: String,
}

fn short(digest: &str) -> String {
    digest.chars().take(19).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_service_images_returns_every_service_with_its_image() {
        let content = r#"
services:
  web:
    image: nginx:latest
  api:
    image: ghcr.io/me/api:1.4.0
  builder:
    build: .
  sidecar:
    image: redis:7
"#;
        let images = extract_service_images(content).expect("parse");
        assert_eq!(
            images,
            vec![
                ComposeServiceImage {
                    name: "web".into(),
                    image: Some("nginx:latest".into()),
                },
                ComposeServiceImage {
                    name: "api".into(),
                    image: Some("ghcr.io/me/api:1.4.0".into()),
                },
                ComposeServiceImage {
                    name: "builder".into(),
                    image: None,
                },
                ComposeServiceImage {
                    name: "sidecar".into(),
                    image: Some("redis:7".into()),
                },
            ]
        );
    }

    #[test]
    fn extract_service_images_rejects_malformed_yaml() {
        assert!(extract_service_images("not: [valid").is_err());
    }

    #[test]
    fn missing_image_errors_are_recognised_across_podman_versions() {
        assert!(is_missing_image_error(
            r#"Error: unable to inspect "docker.io/louislam/uptime-kuma:latest": failed to find image docker.io/louislam/uptime-kuma:latest: docker.io/louislam/uptime-kuma:latest: image not known"#
        ));
        assert!(is_missing_image_error("Error: no such image nginx:latest"));
        assert!(!is_missing_image_error(
            "Error: cannot connect to the podman socket"
        ));
    }

    #[test]
    fn short_truncates_to_the_podman_digest_prefix() {
        let full = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef";
        assert_eq!(short(full), "sha256:0123456789ab");
    }
}
