use super::{check::CoreServiceTargets, stack_env::PreviousCoreTags};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CoreUpdatePhase {
    Preflight,
    Backup,
    Persisted,
    CaddyRecreated,
    DragonflyRecreated,
    Completed,
    RollbackRequired,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreUpdateJournal {
    pub targets: CoreServiceTargets,
    pub previous_tags: PreviousCoreTags,
    pub phase: CoreUpdatePhase,
    pub error: Option<String>,
    pub updated_at: DateTime<Utc>,
}
fn path(dir: &Path) -> PathBuf {
    dir.join("core-update-state.json")
}
#[allow(dead_code)] // used by the upcoming resume-on-restart path.
pub fn read(dir: &Path) -> Option<CoreUpdateJournal> {
    serde_json::from_str(&std::fs::read_to_string(path(dir)).ok()?).ok()
}
pub fn write(dir: &Path, journal: &CoreUpdateJournal) -> std::io::Result<()> {
    let final_path = path(dir);
    let tmp = final_path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(journal)?)?;
    std::fs::rename(tmp, final_path)
}
