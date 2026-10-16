-- Request provenance for each audit row — same "snapshot it now, don't hope
-- we can reconstruct it later" reasoning as 0015_audit_log_target_name and
-- 0017_audit_log_actor_token. Production must answer questions like "where did
-- this delete come from" and "did the same request change three things". The
-- proxy / load balancer log could answer, but it's a different system; copy
-- the bits we need onto the row so the trail is self-contained.
--
-- All columns are nullable. Bare scripts (the seeder), background workers, and
-- pre-migration rows don't have a request to copy from — recording nothing is
-- a more honest answer than inventing one.
ALTER TABLE audit_logs ADD COLUMN source_ip TEXT;
ALTER TABLE audit_logs ADD COLUMN user_agent TEXT;
ALTER TABLE audit_logs ADD COLUMN request_id TEXT;

-- Outcome: success/failure. Same pattern as `actor_token_*`: writes that want
-- to record a failure stamp `outcome='failure'` and `error_message`; writes
-- that don't say stay `NULL` so future reports can distinguish "we don't know
-- the result" from "we know it failed". `error_message` is short (first 500
-- chars of the error before recording); full stack traces belong in logs.
ALTER TABLE audit_logs ADD COLUMN outcome TEXT CHECK (outcome IN ('success', 'failure'));
ALTER TABLE audit_logs ADD COLUMN error_message TEXT;

-- Filters that the API now supports: actor IP (anomalous source) and exact
-- action (build a UI facet "what happened"). Composite with organization_id
-- so the planner can satisfy both via the existing `idx_audit_logs_organization_*`
-- pattern instead of a second table scan.
CREATE INDEX idx_audit_logs_actor_ip ON audit_logs(organization_id, source_ip);
CREATE INDEX idx_audit_logs_action ON audit_logs(organization_id, action);
