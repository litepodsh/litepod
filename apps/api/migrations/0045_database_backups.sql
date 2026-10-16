-- Per-database backup policies and run history.
--
-- A policy is one row per managed database application (identified by its
-- `application_id`); `database_backups` records every run (scheduled or
-- manual) with its S3 object key so history, restore and retention cleanup
-- all resolve through the same table.

CREATE TABLE database_backup_policies (
  application_id TEXT PRIMARY KEY,
  enabled INTEGER NOT NULL DEFAULT 0,
  cron_expression TEXT NOT NULL DEFAULT '0 0 3 * * *',
  retention_count INTEGER NOT NULL DEFAULT 7 CHECK (retention_count >= 1),
  next_run_at INTEGER,
  updated_at INTEGER NOT NULL,
  updated_by TEXT
);

CREATE TABLE database_backups (
  id TEXT PRIMARY KEY,
  application_id TEXT NOT NULL,
  status TEXT NOT NULL,
  engine TEXT NOT NULL,
  started_at INTEGER NOT NULL,
  finished_at INTEGER,
  size_bytes INTEGER,
  s3_key TEXT,
  error_message TEXT
);

CREATE INDEX idx_database_backups_application_started
  ON database_backups(application_id, started_at DESC);