-- OAuth for the MCP endpoint (0068): an MCP client registers itself, the user
-- approves it in the browser, and it gets a token without anyone pasting one.
--
-- Litepod is the authorization server. Every client is public (PKCE, no
-- secret), so a client row is only its name and where codes may be sent.
CREATE TABLE oauth_clients (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    -- JSON array; a redirect must match one entry exactly.
    redirect_uris TEXT NOT NULL,
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%d %H:%M:%S', 'now'))
);

-- One row per approval. Single use: `used_at` is set on exchange, and a
-- second exchange of the same code revokes what the first one produced.
CREATE TABLE oauth_codes (
    code_digest     TEXT PRIMARY KEY,
    client_id       TEXT NOT NULL REFERENCES oauth_clients(id) ON DELETE CASCADE,
    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    redirect_uri    TEXT NOT NULL,
    -- BASE64URL(SHA256(code_verifier)), S256 only.
    code_challenge  TEXT NOT NULL,
    expires_at      TEXT NOT NULL,
    used_at         TEXT
);

-- Rotated on every use. `access_token_id` is the personal_access_tokens row
-- this refresh token stands behind; it is revoked along with it.
CREATE TABLE oauth_refresh_tokens (
    token_digest    TEXT PRIMARY KEY,
    client_id       TEXT NOT NULL REFERENCES oauth_clients(id) ON DELETE CASCADE,
    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    access_token_id TEXT NOT NULL REFERENCES personal_access_tokens(id) ON DELETE CASCADE,
    code_digest     TEXT NOT NULL,
    expires_at      TEXT NOT NULL,
    revoked_at      TEXT
);

CREATE INDEX idx_oauth_refresh_tokens_code_digest ON oauth_refresh_tokens(code_digest);

-- An OAuth access token is an ordinary personal access token that only works
-- on this organization's MCP endpoint. NULL = a normal token, valid anywhere.
ALTER TABLE personal_access_tokens
    ADD COLUMN mcp_organization_id TEXT REFERENCES organizations(id) ON DELETE CASCADE;
