-- The global S3 destination becomes the "default" backup target so the system
-- backup and the per-database backups share one destination model. `s3_config`
-- stays for back-compat reads, but after this migration the single source of
-- truth for "where does the system backup go" is a `backup_s3_targets` row
-- marked `is_default = 1`, carrying the schedule the global config used to own.

ALTER TABLE backup_s3_targets ADD COLUMN is_default INTEGER NOT NULL DEFAULT 0;
ALTER TABLE backup_s3_targets ADD COLUMN schedule_enabled INTEGER NOT NULL DEFAULT 0;
ALTER TABLE backup_s3_targets ADD COLUMN schedule_interval TEXT;
ALTER TABLE backup_s3_targets ADD COLUMN schedule_time TEXT;

-- Seed the default target from the legacy global config when one exists, so an
-- existing install keeps working unchanged after this migration.
INSERT INTO backup_s3_targets
  (id, name, endpoint_url, bucket, region, access_key_id, wrapped_dek, dek_nonce,
   ciphertext, nonce, created_at, updated_at, created_by, is_default,
   schedule_enabled, schedule_interval, schedule_time)
SELECT 'system-default', 'Default (system backups)',
       endpoint_url, bucket, region, access_key_id, wrapped_dek, dek_nonce,
       ciphertext, nonce, updated_at, updated_at, NULL, 1,
       schedule_enabled, schedule_interval, schedule_time
FROM s3_config WHERE id = 1
ON CONFLICT(id) DO NOTHING;

CREATE UNIQUE INDEX idx_backup_s3_targets_default
  ON backup_s3_targets(is_default) WHERE is_default = 1;