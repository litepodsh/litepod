-- Per-application maintenance mode: while `enabled`, every domain of the
-- application answers with a 503 page from Caddy instead of reaching the
-- containers, which keep running.
--
-- One row per application that has touched the feature, created on first write.
-- `page_html` is the visitor's own page; NULL means "use the default template",
-- so a default-page application stores nothing and picks up template fixes.
CREATE TABLE application_maintenance (
    application_id TEXT PRIMARY KEY REFERENCES applications(id) ON DELETE CASCADE,
    enabled        INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0,1)),
    page_html      TEXT,
    updated_at     INTEGER NOT NULL
);
