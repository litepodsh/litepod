-- Anonymous install ping (instance id + version, nothing else) to status.litepod.sh.
-- One row. `instance_id` is a UUID v7 the API fills at first boot; `enabled`
-- defaults to on and the sysadmin is told about it at first login, until
-- `notice_acknowledged` flips.
CREATE TABLE instance_telemetry (
    id                  INTEGER PRIMARY KEY CHECK (id = 1),
    instance_id         TEXT,
    enabled             INTEGER NOT NULL DEFAULT 1,
    notice_acknowledged INTEGER NOT NULL DEFAULT 0,
    updated_at          TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO instance_telemetry (id) VALUES (1);
