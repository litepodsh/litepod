-- The assistant: one conversation per user per organization, the messages in
-- it, and the changes it wants to make.

CREATE TABLE assistant_conversations (
    id              TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title           TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE INDEX idx_assistant_conversations_owner
    ON assistant_conversations(organization_id, user_id, updated_at);

-- `role` is the wire role: user | assistant | tool. Tool messages carry the
-- call id and the JSON the tool returned, so a turn can be replayed to the
-- model exactly as it happened.
CREATE TABLE assistant_messages (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES assistant_conversations(id) ON DELETE CASCADE,
    role            TEXT NOT NULL,
    content         TEXT NOT NULL,
    tool_call_id    TEXT,
    tool_name       TEXT,
    tool_arguments  TEXT,
    created_at      INTEGER NOT NULL
);
CREATE INDEX idx_assistant_messages_conversation
    ON assistant_messages(conversation_id, created_at);

-- A change the assistant wants to make, stored rather than executed.
--
-- The model is never given a tool that mutates infrastructure. A write tool
-- validates its arguments and writes a row here; applying it is a separate
-- authenticated request that re-checks the caller's role against the target
-- application. So an instruction hidden in something the assistant read cannot
-- change a project on its own.
--
-- `arguments` is the tool's validated JSON input, replayed at confirm time, so
-- the row is the whole record of what was proposed.
CREATE TABLE assistant_actions (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES assistant_conversations(id) ON DELETE CASCADE,
    organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    application_id  TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    tool            TEXT NOT NULL,
    arguments       TEXT NOT NULL,
    title           TEXT NOT NULL,
    summary         TEXT NOT NULL,
    -- The place in the UI this change happens, so the panel can take the user there.
    route           TEXT NOT NULL,
    -- pending | applied | rejected | expired | failed
    status          TEXT NOT NULL DEFAULT 'pending',
    proposed_by     TEXT NOT NULL,
    settled_by      TEXT,
    settled_at      INTEGER,
    failure         TEXT,
    created_at      INTEGER NOT NULL,
    expires_at      INTEGER NOT NULL
);
CREATE INDEX idx_assistant_actions_conversation ON assistant_actions(conversation_id, status);
