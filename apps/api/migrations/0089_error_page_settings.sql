-- The page visitors see when Caddy cannot reach an application (502/503/504).
--
-- `error_page_settings.enabled` turns Litepod's default page on or off for the
-- whole instance. On by default.
CREATE TABLE error_page_settings (
    id      INTEGER PRIMARY KEY CHECK (id = 1),
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1))
);

INSERT INTO error_page_settings (id, enabled) VALUES (1, 1);

-- One row per application that changed its error page, created on first write.
-- `enabled = 0` shows Caddy's bare error for this application. `page_html` is the
-- application's own page; NULL means "use the default page", so applications on
-- the default store nothing and pick up template fixes.
CREATE TABLE application_error_page (
    application_id TEXT PRIMARY KEY REFERENCES applications(id) ON DELETE CASCADE,
    enabled        INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    page_html      TEXT,
    updated_at     INTEGER NOT NULL
);
