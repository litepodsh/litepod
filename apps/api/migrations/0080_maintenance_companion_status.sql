-- State of the hidden companion application that builds an uploaded maintenance
-- site with Railpack (see `applications.maintenance_for`).
--
--   none      no companion: the page is the inline HTML, the default, or a plain site
--   building  a Railpack build is queued or running
--   ready     built, and its container is parked (stopped, not removed)
--   failed    the last build failed; `companion_error` says why
--
-- Kept on the parent's row so the UI reads one record.
ALTER TABLE application_maintenance ADD COLUMN companion_status TEXT NOT NULL DEFAULT 'none';
ALTER TABLE application_maintenance ADD COLUMN companion_error TEXT;
