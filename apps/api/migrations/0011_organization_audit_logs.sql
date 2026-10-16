-- Every organization gets the audit-log system available; this column is the
-- per-org on/off switch, off by default until an admin opts in (enforced in
-- the write path, not here — see git/repositories/audit_log.rs).
ALTER TABLE organizations ADD COLUMN audit_logs_enabled INTEGER NOT NULL DEFAULT 0 CHECK (audit_logs_enabled IN (0,1));

-- actor_user_id is nullable with ON DELETE SET NULL, not CASCADE like every
-- other user_id FK in this schema: users.rs::delete_user is a real hard
-- delete, and an audit trail must outlive the actor it records rather than
-- disappear with them (or block their deletion outright).
CREATE TABLE audit_logs (
    id TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    actor_user_id TEXT REFERENCES users(id) ON DELETE SET NULL,
    action TEXT NOT NULL,
    target_type TEXT,
    target_id TEXT,
    metadata TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- `id` is a Uuid::now_v7 (time-ordered), so keyset pagination cursors on it
-- alone with no separate created_at column needed in the index.
CREATE INDEX idx_audit_logs_organization_created ON audit_logs(organization_id, id DESC);
