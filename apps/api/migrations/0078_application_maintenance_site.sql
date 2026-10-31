-- A maintenance page that is a whole small site (an uploaded ZIP with its CSS and
-- images) instead of one HTML document.
--
-- The files live on disk next to the application's other uploads; this column
-- only says "a site is there" and which upload it is (the first 12 hex characters
-- of its SHA-256, like a source archive's id). NULL means the page is the
-- application's inline HTML, or the default template.
ALTER TABLE application_maintenance ADD COLUMN site_id TEXT;
