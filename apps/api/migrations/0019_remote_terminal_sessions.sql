CREATE TABLE remote_terminal_sessions (
    id TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    actor_user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    actor_token_id TEXT,
    actor_token_name TEXT,
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    finished_at TEXT,
    result TEXT CHECK (result IN ('closed', 'disconnected', 'failed')),
    failure_reason TEXT
);
CREATE INDEX idx_remote_terminal_sessions_server_started
    ON remote_terminal_sessions(server_id, id DESC);
