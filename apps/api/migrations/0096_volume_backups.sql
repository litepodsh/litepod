-- Per-volume backups for managed volumes (`application_managed_volumes`).
--
-- Same shape as `database_backup_policies` / `database_backups`, keyed by the
-- volume. The policy goes with its volume; run rows deliberately have no FK so
-- the history (and the S3 key it points at) outlives a deleted volume.

CREATE TABLE volume_backup_policies (
  volume_id TEXT PRIMARY KEY REFERENCES application_managed_volumes(id) ON DELETE CASCADE,
  application_id TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 0,
  cron_expression TEXT NOT NULL DEFAULT '0 0 3 * * *',
  retention_count INTEGER NOT NULL DEFAULT 7 CHECK (retention_count >= 1),
  next_run_at INTEGER,
  updated_at INTEGER NOT NULL,
  updated_by TEXT,
  s3_target_id TEXT
);

CREATE TABLE volume_backups (
  id TEXT PRIMARY KEY,
  volume_id TEXT NOT NULL,
  application_id TEXT NOT NULL,
  status TEXT NOT NULL,
  started_at INTEGER NOT NULL,
  finished_at INTEGER,
  size_bytes INTEGER,
  s3_key TEXT,
  error_message TEXT,
  s3_target_id TEXT
);

CREATE INDEX idx_volume_backups_volume_started
  ON volume_backups(volume_id, started_at DESC);
