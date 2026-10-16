-- The assistant's model connections, one row per key.
--
-- Before this, the assistant had exactly one provider and one key, both frozen
-- at boot from OPENCODE_API_KEY. An organization now brings its own: several
-- connections, each an endpoint plus (usually) a key, each carrying the catalog
-- of models that endpoint actually serves.
--
-- The key never leaves this table in the clear. `api_key_hint` is a redacted
-- display string so the settings list can identify a key without holding it,
-- the same trade notification_providers.config_hint makes.

CREATE TABLE assistant_connections (
    id                TEXT PRIMARY KEY,
    organization_id   TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name              TEXT NOT NULL,
    -- Deliberately unconstrained. `config::PROVIDERS` is the one list of
    -- provider kinds, and every write validates against it; a CHECK here would
    -- mean adding a provider takes a migration as well as a table entry, and
    -- the two would drift. `protocol` below is constrained because it is a
    -- closed set the code actually branches on.
    provider          TEXT NOT NULL,
    -- Which wire we speak, and the only thing that decides it.
    -- 'openai' -> POST {base}/chat/completions, 'anthropic' -> POST {base}/messages.
    protocol          TEXT NOT NULL CHECK (protocol IN ('openai','anthropic')),
    base_url          TEXT NOT NULL,
    -- NULL for Ollama and LM Studio, which want no key at all.
    api_key_encrypted TEXT,
    api_key_hint      TEXT,
    is_default        INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0,1)),
    -- The last probe. `status_detail` carries the provider's own words, because
    -- a 401 and a closed port are fixed in completely different ways.
    status            TEXT NOT NULL DEFAULT 'unverified'
                      CHECK (status IN ('unverified','ok','failed')),
    status_detail     TEXT,
    -- The provider's /models response, normalised:
    -- [{ "id", "label", "context": null|int, "tools": bool,
    --    "blocked": null|"endpoint"|"no-tools" }]
    -- A blocked model is still listed, greyed, with the reason. Not listing it
    -- would leave the user hunting for a model that is right there.
    models_json       TEXT NOT NULL DEFAULT '[]',
    models_fetched_at INTEGER,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);
CREATE UNIQUE INDEX idx_assistant_connections_org_name
    ON assistant_connections(organization_id, name);
-- One default per organization, enforced here rather than in a handler.
CREATE UNIQUE INDEX idx_assistant_connections_default
    ON assistant_connections(organization_id) WHERE is_default = 1;

-- A thread stays on the model it started with, so moving the organization's
-- default does not change the model halfway through an open conversation.
ALTER TABLE assistant_conversations ADD COLUMN connection_id TEXT;
ALTER TABLE assistant_conversations ADD COLUMN model TEXT;
