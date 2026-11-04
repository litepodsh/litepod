-- How long `security_events` rows are kept: 90 days unless a SysAdmin extends it.
CREATE TABLE security_event_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    retention_days INTEGER NOT NULL DEFAULT 90 CHECK (retention_days BETWEEN 90 AND 365)
);
INSERT INTO security_event_settings (id) VALUES (1);
