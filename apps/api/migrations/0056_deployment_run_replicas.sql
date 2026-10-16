-- Group-targeted applications deploy every member server in parallel inside one
-- run. Each log line records which replica wrote it, and each replica's outcome
-- within the run gets its own row, so the run view can show one log per server.

ALTER TABLE application_deployment_run_logs ADD COLUMN server_id TEXT;

CREATE TABLE application_deployment_run_replicas (
    run_id      TEXT NOT NULL REFERENCES application_deployment_runs(id) ON DELETE CASCADE,
    server_id   TEXT NOT NULL,
    server_name TEXT NOT NULL,
    status      TEXT NOT NULL CHECK (status IN ('queued', 'deploying', 'succeeded', 'failed', 'canceled')),
    started_at  INTEGER,
    finished_at INTEGER,
    last_error  TEXT,
    PRIMARY KEY (run_id, server_id)
);

-- A retry of one replica: the fan-out only deploys this member when set.
ALTER TABLE deployment_jobs ADD COLUMN target_server_id TEXT;
