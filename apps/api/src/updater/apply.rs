use std::collections::HashMap;

use futures_util::StreamExt;
use podman_api::{
    Podman, models,
    opts::{
        ContainerCreateOpts, ContainerDeleteOpts, ContainerListOpts, ContainerRestartPolicy,
        PullOpts,
    },
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApplyError {
    #[error("podman client unavailable: {0}")]
    Client(String),
    #[error("could not inspect container: {0}")]
    Inspect(String),
    #[error("pull failed: {0}")]
    Pull(String),
    #[error("could not persist update state: {0}")]
    Persist(String),
    #[error("recreate failed: {0}")]
    Recreate(String),
}

fn client(podman_socket: &str) -> Result<Podman, ApplyError> {
    Podman::new(podman_socket).map_err(|e| ApplyError::Client(e.to_string()))
}

/// The tag the running container was created from, read from
/// `podman inspect .ImageName`.
///
/// This is the authoritative "what version am I" signal. A tag cannot be
/// inherited from a previous container the way an environment variable can,
/// which is what made the old `LITEPOD_VERSION`-based reporting freeze at the
/// version first installed. Returns an empty string when the tag cannot be
/// determined (e.g. native dev with no container), where the caller falls
/// back to the `LITEPOD_VERSION` build arg.
pub async fn current_image_tag(container: &str, podman_socket: &str) -> Result<String, ApplyError> {
    let podman = client(podman_socket)?;
    let info = podman
        .containers()
        .get(container)
        .inspect()
        .await
        .map_err(|e| ApplyError::Inspect(e.to_string()))?;
    let image_name = info.image_name.unwrap_or_default();
    Ok(parse_image_tag(&image_name).unwrap_or_default())
}

/// Quadlet-managed containers carry this label. Recreating one through the Podman
/// API would drop the systemd-inherited sockets it needs, so the updater leaves it
/// to systemd (and the installer).
pub async fn is_systemd_managed(container: &str, podman_socket: &str) -> bool {
    let Ok(podman) = client(podman_socket) else {
        return false;
    };
    podman
        .containers()
        .get(container)
        .inspect()
        .await
        .ok()
        .and_then(|info| info.config)
        .and_then(|config| config.labels)
        .is_some_and(|labels| labels.contains_key("PODMAN_SYSTEMD_UNIT"))
}

/// When the container last started, or `None` if it is missing or not running.
pub async fn container_started_at(
    container: &str,
    podman_socket: &str,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let info = client(podman_socket)
        .ok()?
        .containers()
        .get(container)
        .inspect()
        .await
        .ok()?;
    let state = info.state?;
    if state.running != Some(true) {
        return None;
    }
    state.started_at
}

/// Split the tag off an image reference. Only the segment after the last `/`
/// can carry a tag, so a registry host with a port (`localhost:5000/litepod`)
/// is not mistaken for one. Digest references (`repo@sha256:…`) have no tag.
pub fn parse_image_tag(image: &str) -> Option<String> {
    let last_segment = image.rsplit('/').next()?;
    if last_segment.contains('@') {
        return None;
    }
    let (_, tag) = last_segment.rsplit_once(':')?;
    if tag.is_empty() {
        None
    } else {
        Some(tag.to_string())
    }
}

/// Pull the latest image and recreate the container with the same config.
///
/// This process runs *inside* `container`, so the naive stop-then-create
/// sequence would kill the task before it could start the replacement. We
/// rename the running container out of the way first (a metadata-only op —
/// it keeps running), create and start the replacement under the original
/// name, and force-delete the renamed-away old container once the new one is
/// up. Any failure before that point renames the old container back so the
/// running instance never loses its name.
pub async fn pull_and_recreate<F>(
    container: &str,
    image: &str,
    podman_socket: &str,
    after_start_before_retire: F,
) -> Result<(), ApplyError>
where
    F: FnOnce() -> Result<(), ApplyError>,
{
    let podman = client(podman_socket)?;

    // 1. capture the running container's config while we still have it.
    let info = podman
        .containers()
        .get(container)
        .inspect()
        .await
        .map_err(|e| ApplyError::Inspect(e.to_string()))?;

    // 2. pull the new image. libpod answers HTTP 200 and reports failures as
    // an `error` field *inside* the streamed JSON, so a transport-level Ok is
    // not proof the pull worked. Without this check a failed pull (rate
    // limit, auth, network) silently recreates the container from the stale
    // local image — the update appears to succeed and changes nothing.
    let pull_opts = PullOpts::builder().reference(image).build();
    let images = podman.images();
    let mut pull = images.pull(&pull_opts);
    while let Some(chunk) = pull.next().await {
        let report = chunk.map_err(|e| ApplyError::Pull(e.to_string()))?;
        if let Some(error) = report.error.as_deref().filter(|e| !e.trim().is_empty()) {
            return Err(ApplyError::Pull(error.to_string()));
        }
    }

    // 3. build the replacement's create opts from the captured config. The
    // OLD image's env is needed to tell apart what compose set explicitly
    // from what that image baked in — see `explicit_env`.
    let old_image_env = match info.image.as_deref() {
        Some(id) => image_env(&podman, id).await,
        None => Vec::new(),
    };
    let opts = build_create_opts(container, image, &info, &old_image_env)?;

    // 4. move the running container out of the way, then create + start
    // the replacement under the original name.
    let old_name = format!("{container}-old-{}", chrono::Utc::now().timestamp());
    podman
        .containers()
        .get(container)
        .rename(&old_name)
        .await
        .map_err(|e| ApplyError::Recreate(format!("could not rename old container: {e}")))?;

    if let Err(e) = podman.containers().create(&opts).await {
        let _ = podman.containers().get(&old_name).rename(container).await;
        return Err(ApplyError::Recreate(format!(
            "could not create new container: {e}"
        )));
    }

    let new_container = podman.containers().get(container);
    if let Err(e) = new_container.start(None).await {
        let _ = new_container.remove().await;
        let _ = podman.containers().get(&old_name).rename(container).await;
        return Err(ApplyError::Recreate(format!(
            "could not start new container: {e}"
        )));
    }

    // 5. the replacement is running while the old process is still alive.
    // Persist any state that must survive self-recreation now. If persistence
    // fails, roll back to the old container instead of reporting an update
    // that was never durably recorded.
    if let Err(error) = after_start_before_retire() {
        let _ = new_container.stop(&Default::default()).await;
        let _ = new_container
            .delete(&ContainerDeleteOpts::builder().force(true).build())
            .await;
        let _ = podman.containers().get(&old_name).rename(container).await;
        return Err(error);
    }

    // 6. Clear leftovers from interrupted earlier updates before retiring the
    // current old container. Retiring must be one force-delete call: stopping
    // this container first kills this task before its following delete runs.
    cleanup_retired_containers(&podman, container).await;
    let _ = podman
        .containers()
        .get(&old_name)
        .delete(&ContainerDeleteOpts::builder().force(true).build())
        .await;

    Ok(())
}

fn is_retired_container(name: &str, container: &str) -> bool {
    name.trim_start_matches('/')
        .strip_prefix(&format!("{container}-old-"))
        .is_some_and(|timestamp| timestamp.parse::<i64>().is_ok())
}

async fn cleanup_retired_containers(podman: &Podman, container: &str) {
    let retired = match podman
        .containers()
        .list(&ContainerListOpts::builder().all(true).build())
        .await
    {
        Ok(containers) => containers,
        Err(error) => {
            tracing::warn!(%error, container, "updater: could not list retired containers");
            return;
        }
    };

    for name in retired
        .into_iter()
        .filter(|item| item.exited == Some(true))
        .flat_map(|item| item.names.unwrap_or_default())
        .filter(|name| is_retired_container(name, container))
    {
        let name = name.trim_start_matches('/');
        if let Err(error) = podman
            .containers()
            .get(name)
            .delete(&ContainerDeleteOpts::builder().force(true).build())
            .await
        {
            tracing::warn!(%error, %name, "updater: could not delete retired container");
        }
    }
}

/// The env an image bakes in via `ENV`. Best-effort: if the image cannot be
/// inspected we return empty, which makes `explicit_env` copy everything —
/// the same behaviour as before this change, and safer than dropping vars the
/// container needs to boot.
async fn image_env(podman: &Podman, image: &str) -> Vec<String> {
    match podman.images().get(image).inspect().await {
        Ok(data) => data.config.and_then(|c| c.env).unwrap_or_default(),
        Err(error) => {
            tracing::warn!(
                %error,
                image = %image,
                "updater: could not inspect the old image's env; copying all container env"
            );
            Vec::new()
        }
    }
}

/// Keep only the env entries that were set explicitly on the container,
/// dropping the ones the old image baked in.
///
/// `podman inspect` returns the *merged* env, so copying it wholesale into
/// the replacement container pins the old image's values as container-level
/// overrides — they then win over the new image's own `ENV` forever. That is
/// how an instance updated from v0.1.16 to v0.1.19 kept reporting v0.1.16.
///
/// Entries are compared as whole `KEY=VALUE` strings, so an explicit override
/// of a baked variable (a different value for the same key) is correctly
/// treated as explicit and kept.
fn explicit_env(container_env: &[String], old_image_env: &[String]) -> HashMap<String, String> {
    let baked: std::collections::HashSet<&str> = old_image_env.iter().map(String::as_str).collect();
    container_env
        .iter()
        .filter(|entry| !baked.contains(entry.as_str()))
        .filter_map(|entry| entry.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn build_create_opts(
    name: &str,
    image: &str,
    info: &models::InspectContainerData,
    old_image_env: &[String],
) -> Result<ContainerCreateOpts, ApplyError> {
    let config = info
        .config
        .as_ref()
        .ok_or_else(|| ApplyError::Inspect("missing Config".into()))?;
    let host_config = info
        .host_config
        .as_ref()
        .ok_or_else(|| ApplyError::Inspect("missing HostConfig".into()))?;

    let mut builder = ContainerCreateOpts::builder().name(name).image(image);
    // The Caddy image's default command omits --resume. Preserve the installed
    // config path and keep autosave recovery enabled across core updates.
    if image.starts_with(&format!("{}:", super::core::CADDY_IMAGE_REPO)) {
        let mut command = config.cmd.clone().unwrap_or_else(|| {
            vec![
                "caddy".into(),
                "run".into(),
                "--config".into(),
                "/etc/caddy/Caddyfile".into(),
                "--adapter".into(),
                "caddyfile".into(),
            ]
        });
        if command.first().is_some_and(|arg| arg == "caddy")
            && command.get(1).is_some_and(|arg| arg == "run")
            && !command.iter().any(|arg| arg == "--resume" || arg == "-r")
        {
            command.push("--resume".into());
        }
        builder = builder.command(command);
    }

    if let Some(policy_name) = host_config
        .restart_policy
        .as_ref()
        .and_then(|policy| policy.name.as_deref())
    {
        let policy = match policy_name {
            "always" => Some(ContainerRestartPolicy::Always),
            "on-failure" => Some(ContainerRestartPolicy::OnFailure),
            "unless-stopped" => Some(ContainerRestartPolicy::UnlessStopped),
            _ => None,
        };
        if let Some(policy) = policy {
            builder = builder.restart_policy(policy);
        }
    }

    if let Some(env) = &config.env {
        builder = builder.env(explicit_env(env, old_image_env));
    }

    if let Some(port_bindings) = &host_config.port_bindings {
        let mut mappings = Vec::new();
        for (container_port_proto, bindings) in port_bindings {
            let (port_str, protocol) = container_port_proto
                .split_once('/')
                .unwrap_or((container_port_proto.as_str(), "tcp"));
            let Ok(container_port) = port_str.parse::<u16>() else {
                continue;
            };
            for binding in bindings.iter().flatten() {
                let Some(host_port) = binding
                    .host_port
                    .as_deref()
                    .and_then(|p| p.parse::<u16>().ok())
                else {
                    continue;
                };
                mappings.push(models::PortMapping {
                    container_port: Some(container_port),
                    host_port: Some(host_port),
                    host_ip: binding.host_ip.clone().filter(|ip| !ip.is_empty()),
                    protocol: Some(protocol.to_string()),
                    range: None,
                });
            }
        }
        if !mappings.is_empty() {
            builder = builder.portmappings(mappings);
        }
    }

    if let Some(mounts) = &info.mounts {
        let mut binds = Vec::new();
        let mut volumes = Vec::new();
        for mount in mounts {
            let (Some(source), Some(destination)) = (&mount.source, &mount.destination) else {
                continue;
            };
            match mount.type_.as_deref() {
                Some("volume") => volumes.push(models::NamedVolume {
                    dest: Some(destination.clone()),
                    name: mount.name.clone().or_else(|| Some(source.clone())),
                    is_anonymous: None,
                    options: None,
                }),
                _ => binds.push(models::ContainerMount {
                    source: Some(source.clone()),
                    destination: Some(destination.clone()),
                    options: mount.rw.map(|rw| {
                        vec![if rw {
                            "rw".to_string()
                        } else {
                            "ro".to_string()
                        }]
                    }),
                    _type: Some("bind".to_string()),
                    uid_mappings: None,
                    gid_mappings: None,
                }),
            }
        }
        if !binds.is_empty() {
            builder = builder.mounts(binds);
        }
        if !volumes.is_empty() {
            builder = builder.volumes(volumes);
        }
    }

    if let Some(networks) = info
        .network_settings
        .as_ref()
        .and_then(|settings| settings.networks.as_ref())
        .filter(|networks| !networks.is_empty())
    {
        let mut network_map = HashMap::new();
        for (network_name, details) in networks {
            network_map.insert(
                network_name.clone(),
                models::PerNetworkOptions {
                    aliases: details.aliases.clone(),
                    interface_name: None,
                    static_ips: None,
                    static_mac: None,
                },
            );
        }
        builder = builder
            .net_namespace(models::Namespace {
                nsmode: Some("bridge".into()),
                value: None,
            })
            .networks(network_map);
    }

    Ok(builder.build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caddy_updates_resume_saved_config_and_preserve_custom_config_path() {
        use serde_json::{Value, json};
        for resume in [false, true] {
            let mut command = vec!["caddy", "run", "--config", "/custom/Caddyfile"];
            if resume {
                command.push("--resume");
            }
            let info = serde_json::from_value(json!({
                "Config": {"Cmd": command}, "HostConfig": {},
            }))
            .unwrap();
            let opts = build_create_opts(
                "litepod-caddy",
                "docker.io/library/caddy:2.11.4",
                &info,
                &[],
            )
            .unwrap();
            let body: Value = serde_json::from_str(&opts.serialize().unwrap()).unwrap();
            assert_eq!(
                body["command"],
                json!(["caddy", "run", "--config", "/custom/Caddyfile", "--resume"])
            );
        }
    }

    #[test]
    fn parse_image_tag_reads_the_tag() {
        assert_eq!(
            parse_image_tag("docker.io/litepod/litepod:v0.1.19").as_deref(),
            Some("v0.1.19")
        );
        assert_eq!(parse_image_tag("litepod/litepod:qa").as_deref(), Some("qa"));
    }

    #[test]
    fn parse_image_tag_ignores_a_port_in_the_registry_host() {
        // The colon in `localhost:5000` must not be mistaken for a tag
        // separator — only the segment after the last `/` can carry a tag.
        assert_eq!(
            parse_image_tag("localhost:5000/litepod/litepod:v0.1.19").as_deref(),
            Some("v0.1.19")
        );
        assert_eq!(parse_image_tag("localhost:5000/litepod").as_deref(), None);
    }

    #[test]
    fn parse_image_tag_returns_none_for_untagged_or_digest_refs() {
        assert_eq!(parse_image_tag("docker.io/litepod/litepod"), None);
        assert_eq!(
            parse_image_tag("docker.io/litepod/litepod@sha256:d7788973db25b93f"),
            None
        );
        assert_eq!(parse_image_tag(""), None);
        assert_eq!(parse_image_tag("litepod:"), None);
    }

    #[test]
    fn retired_container_name_is_limited_to_this_updater_prefix() {
        assert!(is_retired_container(
            "/litepod-api-old-1724950000",
            "litepod-api"
        ));
        assert!(!is_retired_container("/litepod-api", "litepod-api"));
        assert!(!is_retired_container("/litepod-api-oldies", "litepod-api"));
        assert!(!is_retired_container(
            "/litepod-api-old-manual",
            "litepod-api"
        ));
        assert!(!is_retired_container(
            "/other-old-1724950000",
            "litepod-api"
        ));
    }

    #[test]
    fn explicit_env_survives_and_image_baked_env_is_dropped() {
        // `podman inspect` returns the *merged* env: everything the image
        // baked in via `ENV` plus whatever compose set explicitly. Copying all
        // of it into the replacement pins the OLD image's values forever —
        // that is exactly how a container updated to v0.1.19 kept reporting
        // LITEPOD_VERSION=v0.1.16.
        let container_env = vec![
            "PATH=/usr/local/bin:/usr/bin".to_string(),
            "LITEPOD_VERSION=v0.1.16".to_string(),
            "APP_ENV=production".to_string(),
            "HASH_SECRET=abc123".to_string(),
        ];
        let old_image_env = vec![
            "PATH=/usr/local/bin:/usr/bin".to_string(),
            "LITEPOD_VERSION=v0.1.16".to_string(),
        ];

        let kept = explicit_env(&container_env, &old_image_env);

        assert_eq!(kept.get("APP_ENV").map(String::as_str), Some("production"));
        assert_eq!(kept.get("HASH_SECRET").map(String::as_str), Some("abc123"));
        assert!(!kept.contains_key("LITEPOD_VERSION"));
        assert!(!kept.contains_key("PATH"));
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn an_explicit_override_of_a_baked_var_is_kept() {
        // Compose setting LITEPOD_IMAGE_TAG to something different from the
        // image default is a deliberate override and must survive.
        let container_env = vec!["LITEPOD_IMAGE_TAG=stable".to_string()];
        let old_image_env = vec!["LITEPOD_IMAGE_TAG=qa".to_string()];

        let kept = explicit_env(&container_env, &old_image_env);

        assert_eq!(
            kept.get("LITEPOD_IMAGE_TAG").map(String::as_str),
            Some("stable")
        );
    }

    #[test]
    fn env_entries_without_an_equals_sign_are_skipped() {
        let kept = explicit_env(&["MALFORMED".to_string()], &[]);
        assert!(kept.is_empty());
    }

    #[test]
    fn an_unknown_old_image_env_keeps_everything() {
        // When the old image cannot be inspected we fall back to copying all
        // env rather than dropping vars the container needs to boot.
        let container_env = vec!["APP_ENV=production".to_string()];
        let kept = explicit_env(&container_env, &[]);
        assert_eq!(kept.get("APP_ENV").map(String::as_str), Some("production"));
    }
}
