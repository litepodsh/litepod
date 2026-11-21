-- Volume backups cover every named volume an application mounts (Compose
-- volumes included), not only managed volumes, so they are keyed by the Podman
-- volume name. 0096 was never released; its tables are empty and replaced.
--
-- Each run records the mount path and Compose service it was taken from, so a
-- restore into a new volume works even while the application is stopped. Run
-- rows have no FK: history (and the S3 key) outlives the application.

DROP TABLE volume_backup_policies;
DROP TABLE volume_backups;

CREATE TABLE volume_backup_policies (
  application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
  volume_name TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 0,
  cron_expression TEXT NOT NULL DEFAULT '0 0 3 * * *',
  retention_count INTEGER NOT NULL DEFAULT 7 CHECK (retention_count >= 1),
  next_run_at INTEGER,
  updated_at INTEGER NOT NULL,
  updated_by TEXT,
  s3_target_id TEXT,
  PRIMARY KEY (application_id, volume_name)
);

CREATE TABLE volume_backups (
  id TEXT PRIMARY KEY,
  application_id TEXT NOT NULL,
  volume_name TEXT NOT NULL,
  mount_path TEXT,
  compose_service TEXT,
  status TEXT NOT NULL,
  triggered_by TEXT NOT NULL,
  started_at INTEGER NOT NULL,
  finished_at INTEGER,
  size_bytes INTEGER,
  s3_key TEXT,
  error_message TEXT,
  s3_target_id TEXT
);

CREATE INDEX idx_volume_backups_volume_started
  ON volume_backups(application_id, volume_name, started_at DESC);
