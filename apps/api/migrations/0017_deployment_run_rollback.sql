-- Rollback: redeploy a previous successful run instead of the current
-- source. A run's own `commit_sha` is already enough for a git-backed deploy
-- (git.rs `deploy_type='git'`, and `deploy_type='git'` with
-- `build_method='compose'`) to re-checkout that exact commit. Two other
-- source kinds have no such link and gain a column here, set only for their
-- own `source_type`:
--   - `artifact_id` (`source_type='static'`) - a run always redeployed
--     whatever `application_deployments.artifact_id` currently points at,
--     never the artifact that was actually live at that run's time.
--   - `compose_content` (`source_type='compose'`, litepod's own managed
--     Compose YAML, not git-backed - see `compose::store::ComposeFileStore`)
--     - the file is a single mutable path on disk with no history at all, so
--     the exact text has to be captured per run or it is gone the moment
--     someone edits and redeploys.
-- `triggered_by` also gains 'manual_rollback' so the history UI can label a
-- rollback distinctly from a plain redeploy.
--
-- application_deployment_runs is the parent of two ON DELETE CASCADE
-- children (application_deployment_run_logs, deployment_jobs). An earlier
-- version of this migration renamed the live table away first
-- (`... RENAME TO application_deployment_runs_v1`) and recreated it under
-- the original name. That is unsafe here: when each migration file runs as
-- its own separate pooled `execute()` call (exactly how the boot-time
-- runner in migrations/runner.rs applies every file), the rename can leave
-- the children's REFERENCES clauses permanently pointing at the renamed-away
-- `_v1` name instead of resolving by name to the table that ends up called
-- `application_deployment_runs` - confirmed by a reproduction test
-- (`migration_0017_rebuild_does_not_cascade_delete_existing_children`) that
-- applies this file the same way the real runner does. The fix is the order
-- SQLite's own docs recommend: build the replacement under a new name, copy
-- data into it, drop the *original* table (never renamed, so nothing else
-- ever pointed away from its name), then rename the replacement into the
-- original's place. The original name is never vacated by a rename, so the
-- unrelated children's `REFERENCES application_deployment_runs(...)` clause
-- is never touched and keeps resolving correctly throughout.
PRAGMA foreign_keys = OFF;

CREATE TABLE application_deployment_runs_next (
    id                   TEXT PRIMARY KEY,
    application_id       TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    source_type          TEXT NOT NULL,
    build_method         TEXT NOT NULL,
    status               TEXT NOT NULL CHECK (status IN ('building', 'deploying', 'succeeded', 'failed', 'canceled')),
    started_at           INTEGER NOT NULL,
    finished_at          INTEGER,
    last_error           TEXT,
    commit_sha           TEXT,
    commit_message       TEXT,
    commit_author_name   TEXT,
    commit_author_email  TEXT,
    triggered_by         TEXT CHECK (triggered_by IN ('manual', 'manual_deploy', 'manual_start', 'manual_stop', 'manual_remove', 'manual_rollback', 'webhook', 'deploy_hook')),
    branch               TEXT,
    created_by           TEXT REFERENCES users(id) ON DELETE SET NULL,
    artifact_id          TEXT REFERENCES application_artifacts(id) ON DELETE SET NULL,
    compose_content      TEXT
);

INSERT INTO application_deployment_runs_next
    (id, application_id, source_type, build_method, status, started_at, finished_at, last_error,
     commit_sha, commit_message, commit_author_name, commit_author_email, triggered_by, branch, created_by)
SELECT id, application_id, source_type, build_method, status, started_at, finished_at, last_error,
       commit_sha, commit_message, commit_author_name, commit_author_email, triggered_by, branch, created_by
FROM application_deployment_runs;

DROP TABLE application_deployment_runs;

ALTER TABLE application_deployment_runs_next RENAME TO application_deployment_runs;

CREATE INDEX idx_deployment_runs_application_started
    ON application_deployment_runs(application_id, started_at DESC);
CREATE INDEX idx_deployment_runs_created_by ON application_deployment_runs(created_by);

PRAGMA foreign_keys = ON;
