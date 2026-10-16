use std::{fmt, fs, os::unix::fs::FileTypeExt};

use podman_api::Podman;

use crate::{dtos::response::AppError, state::state::SharedState};

const ROOTFUL_SOCKET: &str = "/run/podman/podman.sock";

/// Shared client constructor: every handler that talks to Podman builds a
/// fresh client from the resolved socket rather than holding one in state.
pub fn podman_client(state: &SharedState) -> Result<Podman, AppError> {
    Podman::new(&state.podman_socket)
        .map_err(|error| AppError::InternalError(format!("Podman is unavailable: {error}")))
}

pub struct ResolvedPodmanSocket {
    pub endpoint: String,
    pub source: PodmanSocketSource,
}

pub enum PodmanSocketSource {
    Explicit(&'static str),
    XdgRuntimeDir,
    UserRuntimeDir,
    Rootful,
    RootfulDefault,
}

impl fmt::Display for PodmanSocketSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let source = match self {
            Self::Explicit(variable) => variable,
            Self::XdgRuntimeDir => "XDG_RUNTIME_DIR",
            Self::UserRuntimeDir => "effective user runtime directory",
            Self::Rootful => "rootful Podman socket",
            Self::RootfulDefault => "rootful Podman default",
        };
        formatter.write_str(source)
    }
}

trait Environment {
    fn get(&self, key: &str) -> Option<String>;
    fn effective_uid(&self) -> u32;
}

trait SocketPaths {
    fn is_socket(&self, path: &str) -> bool;
}

struct ProcessEnvironment;

impl Environment for ProcessEnvironment {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }

    fn effective_uid(&self) -> u32 {
        unsafe { libc::geteuid() }
    }
}

struct ProcessSocketPaths;

impl SocketPaths for ProcessSocketPaths {
    fn is_socket(&self, path: &str) -> bool {
        fs::symlink_metadata(path)
            .map(|metadata| metadata.file_type().is_socket())
            .unwrap_or(false)
    }
}

pub fn resolve_process_podman_socket() -> ResolvedPodmanSocket {
    resolve_podman_socket(&ProcessEnvironment, &ProcessSocketPaths)
}

fn resolve_podman_socket(
    environment: &impl Environment,
    paths: &impl SocketPaths,
) -> ResolvedPodmanSocket {
    for variable in ["PODMAN_SOCKET", "CONTAINER_HOST"] {
        if let Some(endpoint) = environment
            .get(variable)
            .filter(|value| !value.trim().is_empty())
        {
            return ResolvedPodmanSocket {
                endpoint,
                source: PodmanSocketSource::Explicit(variable),
            };
        }
    }

    let mut candidates = Vec::new();
    if let Some(runtime_dir) = environment
        .get("XDG_RUNTIME_DIR")
        .filter(|value| !value.trim().is_empty())
    {
        candidates.push((
            format!("{runtime_dir}/podman/podman.sock"),
            PodmanSocketSource::XdgRuntimeDir,
        ));
    }
    candidates.push((
        format!(
            "/run/user/{}/podman/podman.sock",
            environment.effective_uid()
        ),
        PodmanSocketSource::UserRuntimeDir,
    ));
    candidates.push((ROOTFUL_SOCKET.into(), PodmanSocketSource::Rootful));

    candidates
        .into_iter()
        .find(|(path, _)| paths.is_socket(path))
        .map(|(path, source)| ResolvedPodmanSocket {
            endpoint: format!("unix://{path}"),
            source,
        })
        .unwrap_or_else(|| ResolvedPodmanSocket {
            endpoint: format!("unix://{ROOTFUL_SOCKET}"),
            source: PodmanSocketSource::RootfulDefault,
        })
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::{Environment, SocketPaths, resolve_podman_socket};

    struct FakeEnvironment {
        values: HashMap<String, String>,
        uid: u32,
    }

    impl FakeEnvironment {
        fn with<const N: usize>(values: [(&str, &str); N]) -> Self {
            Self {
                values: values
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect(),
                uid: 1000,
            }
        }

        fn with_runtime_dir_and_uid(runtime_dir: &str, uid: u32) -> Self {
            let mut environment = Self::with([("XDG_RUNTIME_DIR", runtime_dir)]);
            environment.uid = uid;
            environment
        }

        fn with_uid(uid: u32) -> Self {
            Self {
                values: HashMap::new(),
                uid,
            }
        }
    }

    impl Environment for FakeEnvironment {
        fn get(&self, key: &str) -> Option<String> {
            self.values.get(key).cloned()
        }

        fn effective_uid(&self) -> u32 {
            self.uid
        }
    }

    struct FakeSocketPaths(HashSet<String>);

    impl FakeSocketPaths {
        fn with_existing<const N: usize>(paths: [&str; N]) -> Self {
            Self(paths.into_iter().map(String::from).collect())
        }
    }

    impl SocketPaths for FakeSocketPaths {
        fn is_socket(&self, path: &str) -> bool {
            self.0.contains(path)
        }
    }

    #[test]
    fn explicit_podman_socket_wins_over_all_candidates() {
        let environment = FakeEnvironment::with([
            ("PODMAN_SOCKET", "tcp://podman.example:8080"),
            ("CONTAINER_HOST", "unix:///custom.sock"),
        ]);
        let paths = FakeSocketPaths::with_existing(["/run/user/1000/podman/podman.sock"]);

        assert_eq!(
            resolve_podman_socket(&environment, &paths).endpoint,
            "tcp://podman.example:8080"
        );
    }

    #[test]
    fn container_host_wins_over_automatic_candidates() {
        let environment = FakeEnvironment::with([("CONTAINER_HOST", "unix:///custom.sock")]);
        let paths = FakeSocketPaths::with_existing(["/run/user/1000/podman/podman.sock"]);

        assert_eq!(
            resolve_podman_socket(&environment, &paths).endpoint,
            "unix:///custom.sock"
        );
    }

    #[test]
    fn rootless_runtime_socket_precedes_rootful_socket() {
        let environment = FakeEnvironment::with_runtime_dir_and_uid("/run/user/1000", 1000);
        let paths = FakeSocketPaths::with_existing([
            "/run/user/1000/podman/podman.sock",
            "/run/podman/podman.sock",
        ]);

        assert_eq!(
            resolve_podman_socket(&environment, &paths).endpoint,
            "unix:///run/user/1000/podman/podman.sock"
        );
    }

    #[test]
    fn uid_rootless_socket_precedes_rootful_socket_when_runtime_dir_is_missing() {
        let environment = FakeEnvironment::with_uid(1000);
        let paths = FakeSocketPaths::with_existing([
            "/run/user/1000/podman/podman.sock",
            "/run/podman/podman.sock",
        ]);

        assert_eq!(
            resolve_podman_socket(&environment, &paths).endpoint,
            "unix:///run/user/1000/podman/podman.sock"
        );
    }

    #[test]
    fn rootful_socket_is_used_when_no_rootless_socket_exists() {
        let environment = FakeEnvironment::with_uid(1000);
        let paths = FakeSocketPaths::with_existing(["/run/podman/podman.sock"]);

        assert_eq!(
            resolve_podman_socket(&environment, &paths).endpoint,
            "unix:///run/podman/podman.sock"
        );
    }

    #[test]
    fn rootful_default_is_used_when_no_socket_exists() {
        let environment = FakeEnvironment::with_uid(1000);
        let paths = FakeSocketPaths::with_existing([]);

        assert_eq!(
            resolve_podman_socket(&environment, &paths).endpoint,
            "unix:///run/podman/podman.sock"
        );
    }
}
