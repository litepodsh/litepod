use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::UpdateChannel;

/// The version this instance last installed on itself.
///
/// Persisted in the `litepod_data` volume so it survives container recreation.
/// Without it, a `podman compose up -d` on the `app` service silently reverts
/// the container to whatever `LITEPOD_IMAGE_TAG` says in the host `.env` — a
/// downgrade with no trace. With it, the next check sees the mismatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pin {
    pub pinned_tag: String,
    /// The channel that supplied `pinned_tag`. Missing legacy values are stable.
    #[serde(default)]
    pub channel: UpdateChannel,
    pub applied_at: DateTime<Utc>,
}

pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join("update-state.json")
}

/// Read the pin. A missing, unreadable, or corrupt file is not an error — it
/// means "no pin yet", and the next apply overwrites it. The updater must
/// never fail to boot because of this file.
pub fn read(data_dir: &Path) -> Option<Pin> {
    let raw = std::fs::read_to_string(path(data_dir)).ok()?;
    match serde_json::from_str(&raw) {
        Ok(pin) => Some(pin),
        Err(error) => {
            tracing::warn!(%error, "updater: ignoring corrupt update-state.json");
            None
        }
    }
}

/// Write the pin atomically: temp file in the same directory, then rename.
/// A crash mid-apply must never leave a truncated pin that reads as garbage.
pub fn write(data_dir: &Path, pin: &Pin) -> std::io::Result<()> {
    let final_path = path(data_dir);
    let tmp_path = final_path.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(pin)?;
    std::fs::write(&tmp_path, body)?;
    std::fs::rename(&tmp_path, &final_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updater::UpdateChannel;

    /// A scratch directory that cleans itself up. Avoids pulling in `tempfile`
    /// just for these tests — the constraint is no new dependencies.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("litepod-pin-test-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn read_returns_none_when_no_pin_was_ever_written() {
        let scratch = Scratch::new("missing");
        assert!(read(&scratch.0).is_none());
    }

    #[test]
    fn write_then_read_round_trips() {
        let scratch = Scratch::new("roundtrip");
        let pin = Pin {
            pinned_tag: "v0.1.19".into(),
            channel: UpdateChannel::Stable,
            applied_at: Utc::now(),
        };
        write(&scratch.0, &pin).unwrap();

        let loaded = read(&scratch.0).unwrap();
        assert_eq!(loaded.pinned_tag, "v0.1.19");
        assert_eq!(loaded.channel, UpdateChannel::Stable);
        assert_eq!(loaded.applied_at.timestamp(), pin.applied_at.timestamp());
    }

    #[test]
    fn write_overwrites_a_previous_pin() {
        let scratch = Scratch::new("overwrite");
        write(
            &scratch.0,
            &Pin {
                pinned_tag: "v0.1.16".into(),
                channel: UpdateChannel::Stable,
                applied_at: Utc::now(),
            },
        )
        .unwrap();
        write(
            &scratch.0,
            &Pin {
                pinned_tag: "v0.1.19".into(),
                channel: UpdateChannel::Stable,
                applied_at: Utc::now(),
            },
        )
        .unwrap();

        assert_eq!(read(&scratch.0).unwrap().pinned_tag, "v0.1.19");
    }

    #[test]
    fn corrupt_pin_reads_as_absent_instead_of_panicking() {
        let scratch = Scratch::new("corrupt");
        std::fs::write(path(&scratch.0), b"{truncated").unwrap();
        assert!(read(&scratch.0).is_none());
    }

    #[test]
    fn write_leaves_no_temp_file_behind() {
        let scratch = Scratch::new("notmp");
        write(
            &scratch.0,
            &Pin {
                pinned_tag: "v0.1.19".into(),
                channel: UpdateChannel::Stable,
                applied_at: Utc::now(),
            },
        )
        .unwrap();

        let leftovers: Vec<_> = std::fs::read_dir(&scratch.0)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "left temp files: {leftovers:?}");
    }

    #[test]
    fn write_then_read_preserves_the_channel() {
        let scratch = Scratch::new("channel");
        write(
            &scratch.0,
            &Pin {
                pinned_tag: "v0.1.23-alpha.2".into(),
                channel: UpdateChannel::Alpha,
                applied_at: Utc::now(),
            },
        )
        .unwrap();

        assert_eq!(read(&scratch.0).unwrap().channel, UpdateChannel::Alpha);
    }
}
