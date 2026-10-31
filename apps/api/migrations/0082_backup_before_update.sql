-- When set, applying a Litepod update first takes an instance backup to the default target and
-- stops if that backup fails. Lives on the default target next to its schedule.
ALTER TABLE backup_s3_targets ADD COLUMN backup_before_update INTEGER NOT NULL DEFAULT 0;
