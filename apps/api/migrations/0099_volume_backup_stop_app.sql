-- Optional consistency mode for volume backups: stop the application while the
-- volume is exported (databases such as Postgres cannot be copied safely while
-- they write), then start it again.
ALTER TABLE volume_backup_policies ADD COLUMN stop_app INTEGER NOT NULL DEFAULT 0;
