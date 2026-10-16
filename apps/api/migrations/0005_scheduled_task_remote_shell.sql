-- The scheduled-tasks feature gains a `remote_shell` runner that executes the
-- script directly on a remote server host over SSH (no container). SQLite
-- cannot alter a CHECK constraint in place, so both tables are rebuilt with the
-- expanded runner allowlist.
PRAGMA foreign_keys = OFF;

BEGIN;

CREATE TABLE scheduled_tasks_new (
    id                   TEXT PRIMARY KEY,
    organization_id      TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    server_id            TEXT REFERENCES servers(id) DEFAULT 'local',
    name                 TEXT NOT NULL,
    description          TEXT NOT NULL DEFAULT '',
    cron_expression      TEXT NOT NULL,
    runner               TEXT NOT NULL CHECK (runner IN ('ephemeral', 'remote_shell', 'litepod_api')),
    image                TEXT,
    shell                TEXT NOT NULL CHECK (shell IN ('bash', 'sh')),
    script               TEXT NOT NULL,
    timeout_seconds      INTEGER NOT NULL CHECK (timeout_seconds BETWEEN 1 AND 86400),
    network_access       INTEGER NOT NULL DEFAULT 0 CHECK (network_access IN (0, 1)),
    enabled              INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    next_run_at          INTEGER,
    active_execution_id  TEXT,
    created_by           TEXT NOT NULL REFERENCES users(id),
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    CHECK ((runner = 'ephemeral' AND image IS NOT NULL AND length(trim(image)) > 0) OR (runner = 'litepod_api' AND image IS NULL) OR (runner = 'remote_shell' AND image IS NULL))
);

INSERT INTO scheduled_tasks_new SELECT * FROM scheduled_tasks;

DROP TABLE scheduled_tasks;

ALTER TABLE scheduled_tasks_new RENAME TO scheduled_tasks;

CREATE INDEX idx_scheduled_tasks_due ON scheduled_tasks(enabled, next_run_at);
CREATE INDEX idx_scheduled_tasks_server ON scheduled_tasks(server_id);
CREATE INDEX idx_scheduled_tasks_organization ON scheduled_tasks(organization_id, created_at DESC);

CREATE TABLE scheduled_task_executions_new (
    id                TEXT PRIMARY KEY,
    task_id           TEXT NOT NULL REFERENCES scheduled_tasks(id) ON DELETE CASCADE,
    organization_id   TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    server_id         TEXT REFERENCES servers(id) DEFAULT 'local',
    trigger           TEXT NOT NULL CHECK (trigger IN ('scheduled', 'manual')),
    runner            TEXT NOT NULL CHECK (runner IN ('ephemeral', 'remote_shell', 'litepod_api')),
    image             TEXT,
    image_id          TEXT,
    shell             TEXT NOT NULL CHECK (shell IN ('bash', 'sh')),
    script            TEXT NOT NULL,
    timeout_seconds   INTEGER NOT NULL,
    network_access    INTEGER NOT NULL DEFAULT 0 CHECK (network_access IN (0, 1)),
    status            TEXT NOT NULL CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'canceled', 'replaced', 'interrupted')),
    container_name    TEXT,
    container_id      TEXT,
    process_group_id  INTEGER,
    queued_at         INTEGER NOT NULL,
    started_at        INTEGER,
    finished_at       INTEGER,
    exit_code         INTEGER,
    error             TEXT,
    log_bytes         INTEGER NOT NULL DEFAULT 0,
    logs_truncated    INTEGER NOT NULL DEFAULT 0 CHECK (logs_truncated IN (0, 1))
);

INSERT INTO scheduled_task_executions_new SELECT * FROM scheduled_task_executions;

DROP TABLE scheduled_task_executions;

ALTER TABLE scheduled_task_executions_new RENAME TO scheduled_task_executions;

CREATE INDEX idx_scheduled_task_executions_task ON scheduled_task_executions(task_id, queued_at DESC);
CREATE INDEX idx_scheduled_task_executions_server ON scheduled_task_executions(server_id);
CREATE INDEX idx_scheduled_task_executions_organization
    ON scheduled_task_executions(organization_id, queued_at DESC);

COMMIT;

PRAGMA foreign_keys = ON;
