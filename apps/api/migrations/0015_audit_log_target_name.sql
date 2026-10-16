-- A human-readable snapshot of what target_id pointed to at the time of the
-- action (e.g. a project's name). target_id alone is useless once the row it
-- names is gone — deletions are exactly the entries where this matters most —
-- and a live join can't recover a name that no longer exists.
ALTER TABLE audit_logs ADD COLUMN target_name TEXT;
