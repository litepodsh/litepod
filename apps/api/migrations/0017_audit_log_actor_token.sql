-- Which personal access token made a change, not just which user — same
-- "snapshot it as its own column" reasoning as target_name in
-- 0015_audit_log_target_name.sql: a token can be renamed or revoked later,
-- and the entry should still say what it was called at the time.
ALTER TABLE audit_logs ADD COLUMN actor_token_id TEXT;
ALTER TABLE audit_logs ADD COLUMN actor_token_name TEXT;
