-- Whole-application volume backups: every volume copied in one run (one stop
-- of the app) shares a `group_id`, so the run can be restored as a set.
-- NULL for single-volume runs.
ALTER TABLE volume_backups ADD COLUMN group_id TEXT;

CREATE INDEX idx_volume_backups_group ON volume_backups(application_id, group_id);
