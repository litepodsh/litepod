-- Named S3 destinations for database backups.
--
-- A policy may point at one of these targets; when `s3_target_id` is NULL the
-- policy falls back to the global S3 destination (`s3_config.id = 1`). Each
-- `database_backups` row records the target it was uploaded to, so restore,
-- retention cleanup and delete resolve the exact destination instead of
-- assuming the current global one.

CREATE TABLE backup_s3_targets (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  endpoint_url TEXT NOT NULL,
  bucket TEXT NOT NULL,
  region TEXT NOT NULL DEFAULT 'us-east-1',
  access_key_id TEXT NOT NULL,
  -- Secret access key, encrypted at rest (same shape as `s3_config`).
  wrapped_dek       BLOB NOT NULL,
  dek_nonce         BLOB NOT NULL,
  ciphertext        BLOB NOT NULL,
  nonce             BLOB NOT NULL,
  created_at        INTEGER NOT NULL,
  updated_at        INTEGER NOT NULL,
  created_by        TEXT
);

CREATE UNIQUE INDEX idx_backup_s3_targets_name ON backup_s3_targets(name);

ALTER TABLE database_backup_policies ADD COLUMN s3_target_id TEXT;

ALTER TABLE database_backups ADD COLUMN s3_target_id TEXT;

CREATE INDEX idx_database_backups_target ON database_backups(s3_target_id);