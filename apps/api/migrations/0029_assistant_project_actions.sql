-- Project-scoped assistant actions: creating, renaming and deleting a project
-- have no application to hang off, so `application_id` stops being mandatory.
--
-- SQLite cannot relax NOT NULL in place, so the table is rebuilt the same way
-- 0017 does it: build the replacement under a new name, copy, drop the
-- original (never renamed, so nothing else ever points away from its name),
-- then rename the replacement into its place. `foreign_keys` is off across the
-- rebuild so the drop does not cascade the rows away.
--
-- `project_id` still carries no foreign key: rows written before 0027 default
-- it to the empty string, which no projects row can satisfy.
PRAGMA foreign_keys = OFF;

CREATE TABLE assistant_actions_next (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES assistant_conversations(id) ON DELETE CASCADE,
    organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    -- NULL for an action about a project rather than one application.
    application_id  TEXT REFERENCES applications(id) ON DELETE CASCADE,
    tool            TEXT NOT NULL,
    arguments       TEXT NOT NULL,
    title           TEXT NOT NULL,
    summary         TEXT NOT NULL,
    route           TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending',
    proposed_by     TEXT NOT NULL,
    settled_by      TEXT,
    settled_at      INTEGER,
    failure         TEXT,
    created_at      INTEGER NOT NULL,
    expires_at      INTEGER NOT NULL,
    project_id      TEXT NOT NULL DEFAULT '',
    environment_id  TEXT NOT NULL DEFAULT ''
);

INSERT INTO assistant_actions_next
    (id, conversation_id, organization_id, application_id, tool, arguments, title, summary,
     route, status, proposed_by, settled_by, settled_at, failure, created_at, expires_at,
     project_id, environment_id)
SELECT id, conversation_id, organization_id, application_id, tool, arguments, title, summary,
       route, status, proposed_by, settled_by, settled_at, failure, created_at, expires_at,
       project_id, environment_id
FROM assistant_actions;

DROP TABLE assistant_actions;

ALTER TABLE assistant_actions_next RENAME TO assistant_actions;

CREATE INDEX idx_assistant_actions_conversation ON assistant_actions(conversation_id, status);

PRAGMA foreign_keys = ON;
