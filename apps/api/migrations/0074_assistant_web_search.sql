-- The web search an organization lets its assistant and MCP agents use.
--
-- One row per organization: a SearXNG instance the organization runs, or a
-- Brave Search API key. The key is encrypted with the same AES-256-GCM helper
-- the git providers use and is write-only: nothing returns it, not even a hint.
-- A key is replaced by deleting the row and saving a new one.
CREATE TABLE assistant_web_search (
    organization_id   TEXT PRIMARY KEY REFERENCES organizations(id) ON DELETE CASCADE,
    provider          TEXT NOT NULL CHECK (provider IN ('searxng','brave')),
    -- A SearXNG instance's address. Fixed for Brave, kept so the row is uniform.
    base_url          TEXT NOT NULL,
    api_key_encrypted TEXT,
    enabled           INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);
