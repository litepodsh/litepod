-- Real pipeline steps per deployment run (clone, build, start, health check,
-- route, ...). The worker plans a run's steps as `pending`, then moves them
-- through `running` to a terminal state. `server_id` is '' for a single-server
-- run and the replica's server for a group fan-out, so parallel replicas keep
-- their own timeline.
CREATE TABLE application_deployment_run_steps (
    run_id      TEXT NOT NULL REFERENCES application_deployment_runs(id) ON DELETE CASCADE,
    server_id   TEXT NOT NULL DEFAULT '',
    position    INTEGER NOT NULL,
    name        TEXT NOT NULL,
    status      TEXT NOT NULL CHECK (status IN ('pending', 'running', 'succeeded', 'failed', 'skipped')),
    started_at  INTEGER,
    finished_at INTEGER,
    PRIMARY KEY (run_id, server_id, name)
);
