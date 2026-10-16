-- SSO session controls (docs/superpowers/specs/2026-10-08-sso-session-controls-design.md).
-- Additive only. Default TTL (3 days) keeps existing connections' behavior.
ALTER TABLE sso_connections ADD COLUMN session_ttl_minutes INTEGER NOT NULL DEFAULT 4320;

ALTER TABLE sso_login_states ADD COLUMN login_hint TEXT;
ALTER TABLE sso_login_states ADD COLUMN forced INTEGER NOT NULL DEFAULT 0;

ALTER TABLE sessions ADD COLUMN sso_connection_id TEXT;
ALTER TABLE sessions ADD COLUMN sso_subject TEXT;
ALTER TABLE sessions ADD COLUMN id_token TEXT;
CREATE INDEX idx_sessions_sso ON sessions(sso_connection_id, sso_subject);
