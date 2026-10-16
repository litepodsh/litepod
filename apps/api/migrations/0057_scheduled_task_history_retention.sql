-- Configurable retention window for scheduled task run history (executions and
-- their logs). 0 keeps history forever; the default matches the previous fixed
-- 60-day purge.

CREATE TABLE scheduled_task_settings (
    id                     INTEGER PRIMARY KEY CHECK (id = 1),
    history_retention_days INTEGER NOT NULL DEFAULT 60 CHECK (history_retention_days >= 0)
);

INSERT INTO scheduled_task_settings (id, history_retention_days) VALUES (1, 60);
