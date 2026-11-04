-- One-time links to an application's uploaded source ZIP, so an MCP agent in a
-- later session can fetch the project with `curl`. Only the hash of the token is
-- kept, and redeeming a link deletes its row.
CREATE TABLE source_archive_downloads (
    token_hash     TEXT PRIMARY KEY,
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    expires_at     INTEGER NOT NULL
);
