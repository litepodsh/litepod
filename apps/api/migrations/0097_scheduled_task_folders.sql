-- Optional folder that groups scheduled tasks in the list. Empty means ungrouped;
-- a folder exists only while some task names it.
ALTER TABLE scheduled_tasks ADD COLUMN folder TEXT NOT NULL DEFAULT '';
