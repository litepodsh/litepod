use std::{fs, io::Write, path::PathBuf};

use thiserror::Error;

use super::check::CoreServiceTargets;

#[derive(Debug, Error)]
pub enum StackEnvError {
    #[error("persistent core updates are unavailable; run the installer once")]
    Unavailable,
    #[error("could not update stack environment: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PreviousCoreTags {
    pub caddy: Option<String>,
    pub dragonfly: Option<String>,
}

pub struct StackEnv {
    path: PathBuf,
}

impl StackEnv {
    pub fn open(path: PathBuf) -> Result<Self, StackEnvError> {
        if path.is_file() {
            Ok(Self { path })
        } else {
            Err(StackEnvError::Unavailable)
        }
    }

    pub fn replace_core_tags(
        &self,
        targets: &CoreServiceTargets,
    ) -> Result<PreviousCoreTags, StackEnvError> {
        let raw = fs::read_to_string(&self.path)?;
        let previous = PreviousCoreTags {
            caddy: value(&raw, "CADDY_IMAGE_TAG"),
            dragonfly: value(&raw, "DRAGONFLY_IMAGE_TAG"),
        };
        write_atomic(
            &self.path,
            &replace(
                &replace(&raw, "CADDY_IMAGE_TAG", &targets.caddy),
                "DRAGONFLY_IMAGE_TAG",
                &targets.dragonfly,
            ),
        )?;
        Ok(previous)
    }

    /// Keep the host `.env`'s `LITEPOD_IMAGE_TAG` in sync with the tag the
    /// `app` container was just recreated with. Without this, a later
    /// `podman compose up -d` on the `app` service reads the stale tag left
    /// over from install and silently reverts the container to it — see
    /// `pin.rs`'s doc comment for the incident this closes.
    pub fn set_app_tag(&self, tag: &str) -> Result<(), StackEnvError> {
        let raw = fs::read_to_string(&self.path)?;
        write_atomic(&self.path, &replace(&raw, "LITEPOD_IMAGE_TAG", tag))?;
        Ok(())
    }

    pub fn restore(&self, previous: &PreviousCoreTags) -> Result<(), StackEnvError> {
        let raw = fs::read_to_string(&self.path)?;
        let caddy = previous.caddy.as_deref().unwrap_or("2.11.4");
        let dragonfly = previous.dragonfly.as_deref().unwrap_or("v1.39.0");
        write_atomic(
            &self.path,
            &replace(
                &replace(&raw, "CADDY_IMAGE_TAG", caddy),
                "DRAGONFLY_IMAGE_TAG",
                dragonfly,
            ),
        )?;
        Ok(())
    }
}

/// Keys a backup's `stack.env` contributes on restore. The database and
/// encrypted columns only make sense with the secrets they were written
/// with; everything else (public IP/URL, image tags, update channel) belongs
/// to the host doing the restore — adopting an old `LITEPOD_IMAGE_TAG` would
/// downgrade the binary below the restored schema on the next `compose up`.
const RESTORED_SECRET_KEYS: &[&str] = &[
    "HASH_SECRET",
    "ENCRYPTION_KEY",
    "METRICS_USERNAME",
    "METRICS_PASSWORD",
    "DFLY_PASSWORD",
    "RESEND_API_KEY",
    "TURNSTILE_SECRET",
];

/// Merges a backup's `stack.env` into the host's at `path`: secrets from the
/// backup, every other line kept. With no host file there is nothing to keep,
/// so the backup's file is written as-is.
pub fn adopt_secrets(path: &std::path::Path, backup: &str) -> Result<(), std::io::Error> {
    let path = path.to_path_buf();
    if !path.is_file() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        return write_atomic(&path, backup);
    }
    let mut raw = fs::read_to_string(&path)?;
    for key in RESTORED_SECRET_KEYS {
        if let Some(secret) = value(backup, key) {
            raw = replace(&raw, key, &secret);
        }
    }
    write_atomic(&path, &raw)
}

fn value(raw: &str, key: &str) -> Option<String> {
    raw.lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")).map(str::to_owned))
}

fn replace(raw: &str, key: &str, target: &str) -> String {
    let mut found = false;
    let mut out: Vec<String> = raw
        .lines()
        .filter_map(|line| {
            if line.starts_with(&format!("{key}=")) {
                if found {
                    return None;
                }
                found = true;
                Some(format!("{key}={target}"))
            } else {
                Some(line.to_owned())
            }
        })
        .collect();
    if !found {
        out.push(format!("{key}={target}"));
    }
    format!("{}\n", out.join("\n"))
}

fn write_atomic(path: &PathBuf, body: &str) -> Result<(), std::io::Error> {
    let tmp = path.with_extension("env.tmp");
    let mut file = fs::File::create(&tmp)?;
    file.write_all(body.as_bytes())?;
    file.sync_all()?;
    fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adopt_secrets_takes_secrets_from_backup_and_keeps_host_values() {
        let dir = std::env::temp_dir().join(format!("litepod-adopt-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env");
        fs::write(
            &path,
            "HASH_SECRET=new-hash\nENCRYPTION_KEY=new-key\nLITEPOD_PUBLIC_IP=10.0.0.2\nLITEPOD_IMAGE_TAG=v0.1.80\n",
        )
        .unwrap();

        adopt_secrets(
            &path,
            "HASH_SECRET=old-hash\nENCRYPTION_KEY=old-key\nDFLY_PASSWORD=old-dfly\nLITEPOD_PUBLIC_IP=10.0.0.1\nLITEPOD_IMAGE_TAG=v0.1.60\n",
        )
        .unwrap();

        let merged = fs::read_to_string(&path).unwrap();
        assert_eq!(value(&merged, "HASH_SECRET").as_deref(), Some("old-hash"));
        assert_eq!(value(&merged, "ENCRYPTION_KEY").as_deref(), Some("old-key"));
        assert_eq!(value(&merged, "DFLY_PASSWORD").as_deref(), Some("old-dfly"));
        assert_eq!(
            value(&merged, "LITEPOD_PUBLIC_IP").as_deref(),
            Some("10.0.0.2")
        );
        assert_eq!(
            value(&merged, "LITEPOD_IMAGE_TAG").as_deref(),
            Some("v0.1.80")
        );
        let _ = fs::remove_dir_all(dir);
    }
    #[test]
    fn updates_only_core_tags_and_restores_previous_values() {
        let dir = std::env::temp_dir().join("litepod-stack-env-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env");
        fs::write(
            &path,
            "HASH_SECRET=keep\nCADDY_IMAGE_TAG=2.11.3\nDRAGONFLY_IMAGE_TAG=v1.38.0\n# keep\n",
        )
        .unwrap();
        let env = StackEnv::open(path.clone()).unwrap();
        let previous = env
            .replace_core_tags(&CoreServiceTargets {
                caddy: "2.11.4".into(),
                dragonfly: "v1.39.0".into(),
            })
            .unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "HASH_SECRET=keep\nCADDY_IMAGE_TAG=2.11.4\nDRAGONFLY_IMAGE_TAG=v1.39.0\n# keep\n"
        );
        env.restore(&previous).unwrap();
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("CADDY_IMAGE_TAG=2.11.3")
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn set_app_tag_updates_only_the_app_tag() {
        let dir = std::env::temp_dir().join("litepod-stack-env-app-tag-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env");
        fs::write(
            &path,
            "HASH_SECRET=keep\nLITEPOD_IMAGE_TAG=v0.1.23\nCADDY_IMAGE_TAG=2.11.4\n",
        )
        .unwrap();
        let env = StackEnv::open(path.clone()).unwrap();
        env.set_app_tag("v0.1.37").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "HASH_SECRET=keep\nLITEPOD_IMAGE_TAG=v0.1.37\nCADDY_IMAGE_TAG=2.11.4\n"
        );
        let _ = fs::remove_dir_all(dir);
    }
}
