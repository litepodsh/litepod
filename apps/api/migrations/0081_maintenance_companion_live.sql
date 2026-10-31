-- Whether routes may point at the companion's container right now.
--
-- Set only after the parked container was started and answered its health
-- check, and cleared before it is parked again, so a route never names a
-- container that is not serving. While 0 (or when the companion is gone), an
-- uploaded site is served by the API from its static copy instead.
ALTER TABLE application_maintenance ADD COLUMN companion_live INTEGER NOT NULL DEFAULT 0;
