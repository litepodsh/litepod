-- Append-only lifecycle audit for container exec (terminal) sessions.
--
-- Container exec is full control ("total access to whatever the container can
-- reach — env secrets, mounted volumes, pivot"), so it gets the same
-- mandatory, toggle-proof session trail as `remote_terminal_sessions`
-- (migration 0019) rather than the optional `audit_logs` table. Deliberately
-- write-only: sessions are recorded for forensics and the planned reporting
-- interface, never surfaced as a mutable resource.
--
-- Exactly one of application_id / server_id is set: application exec is scoped
-- by application, server-console exec by server. Never store what was run, its
-- arguments, credentials, or any terminal payload — lifecycle metadata only.

CREATE TABLE container_exec_sessions (
    id TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    application_id TEXT REFERENCES applications(id) ON DELETE CASCADE,
    server_id TEXT REFERENCES servers(id) ON DELETE CASCADE,
    container_id TEXT NOT NULL,
    actor_user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    actor_token_id TEXT,
    actor_token_name TEXT,
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    finished_at TEXT,
    result TEXT CHECK (result IN ('closed', 'disconnected', 'failed')),
    failure_reason TEXT,
    CHECK ((application_id IS NULL) != (server_id IS NULL))
);
CREATE INDEX idx_container_exec_sessions_org_started
    ON container_exec_sessions(organization_id, id DESC);