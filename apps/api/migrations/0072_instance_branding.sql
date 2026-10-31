-- Enterprise white-label: the name, logo and favicon a self-hosted instance
-- shows instead of Litepod's. One row, NULL columns mean "use the default".
CREATE TABLE instance_branding (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    name       TEXT,
    logo_url   TEXT,
    favicon_url TEXT,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO instance_branding (id) VALUES (1);
