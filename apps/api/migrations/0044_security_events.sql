-- Append-only security and sysadmin operation trail.
--
-- Companion to `audit_logs` (org-scoped, toggle-gated) for events that have no
-- clean organization to hang off:
--
--   * account security self-service — credential changes, two-factor toggles,
--     backup-code regeneration, session and access-value revocation
--   * sysadmin operations — backups, restores, maintenance prunes, certificate
--     inventory changes, infrastructure restarts, system updates
--
-- Like `remote_terminal_sessions` and `container_exec_sessions`, this table is
-- deliberate, write-only forensics: lifecycle metadata and an action name,
-- never credentials. Nothing that can authenticate is stored — `user_id` is
-- the *subject* account (the account whose credential changed, whose access
-- value was revoked, or the user a sysadmin deleted), and `actor_user_id` is
-- who performed it.

CREATE TABLE security_events (
    id TEXT PRIMARY KEY,
    user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    actor_user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    actor_token_id TEXT,
    actor_token_name TEXT,
    action TEXT NOT NULL,
    metadata TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_security_events_user_created
    ON security_events(user_id, id DESC);
CREATE INDEX idx_security_events_actor_created
    ON security_events(actor_user_id, id DESC);