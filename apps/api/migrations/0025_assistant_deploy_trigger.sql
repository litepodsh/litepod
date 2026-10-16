-- `triggered_by` gains `assistant_deploy`, so a run the assistant queued says so
-- in the history instead of being indistinguishable from a button press.
--
-- SQLite cannot widen a CHECK in place, so this is the same table rebuild
-- migration 0017 did for `manual_rollback`: new table, copy, drop, rename.
-- Foreign keys are off for the swap because the children (`deployment_jobs`,
-- `application_deployment_run_logs`) point at this table by name; the name is
-- never absent from a committed state, so they keep resolving.
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
    triggered_by         TEXT CHECK (triggered_by IN ('manual', 'manual_deploy', 'manual_start', 'manual_stop', 'manual_remove', 'manual_rollback', 'assistant_deploy', 'webhook', 'deploy_hook')),
    branch               TEXT,
    created_by           TEXT REFERENCES users(id) ON DELETE SET NULL,
    artifact_id          TEXT REFERENCES application_artifacts(id) ON DELETE SET NULL,
    compose_content      TEXT
);

INSERT INTO application_deployment_runs_next
    (id, application_id, source_type, build_method, status, started_at, finished_at, last_error,
     commit_sha, commit_message, commit_author_name, commit_author_email, triggered_by, branch,
     created_by, artifact_id, compose_content)
SELECT id, application_id, source_type, build_method, status, started_at, finished_at, last_error,
       commit_sha, commit_message, commit_author_name, commit_author_email, triggered_by, branch,
       created_by, artifact_id, compose_content
FROM application_deployment_runs;

DROP TABLE application_deployment_runs;

ALTER TABLE application_deployment_runs_next RENAME TO application_deployment_runs;

CREATE INDEX idx_deployment_runs_application_started
    ON application_deployment_runs(application_id, started_at DESC);
CREATE INDEX idx_deployment_runs_created_by ON application_deployment_runs(created_by);

PRAGMA foreign_keys = ON;
