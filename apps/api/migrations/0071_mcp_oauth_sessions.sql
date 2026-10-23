-- One record per MCP OAuth login: the moment a user approves an agent (Claude,
-- ChatGPT, Hermes, ...) on the consent page, a session row is written here and
-- stays listed while the grant is open. The grant's refresh chain rotates, so
-- `expires_at` moves forward with every rotation; when the chain dies (revoked,
-- replayed, or left to lapse) the row is marked closed and kept as history.
CREATE TABLE mcp_oauth_sessions (
    id              TEXT PRIMARY KEY,
    -- One grant per code: every rotation of the same approval shares it.
    code_digest     TEXT NOT NULL UNIQUE,
    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    client_id       TEXT NOT NULL REFERENCES oauth_clients(id) ON DELETE CASCADE,
    -- Snapshot of the client name at login; the client row may change later.
    client_name     TEXT NOT NULL,
    -- Detected from the client name and redirect URIs:
    -- claude | chatgpt | hermes | other.
    agent           TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%S', 'now')),
    expires_at      TEXT NOT NULL,
    closed_at       TEXT
);

CREATE INDEX idx_mcp_oauth_sessions_user_id ON mcp_oauth_sessions(user_id);
