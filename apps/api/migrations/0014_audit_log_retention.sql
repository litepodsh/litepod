-- Per-org retention window for audit_logs (audit_logs.sql, migration 0011).
-- Capped at 365 days; enforced here and re-checked in
-- cqrs::commands::organization::update_organization so a bad value 400s
-- instead of hitting the CHECK constraint as a 500.
ALTER TABLE organizations ADD COLUMN audit_logs_retention_days INTEGER NOT NULL DEFAULT 90 CHECK (audit_logs_retention_days BETWEEN 1 AND 365);
