-- The server's IANA zone, so the fleet map can show its own wall clock.
-- Split from 0032 because that migration had already been applied: the runner
-- records a file by name and never reruns it, so a column added to a shipped
-- migration silently never exists.
ALTER TABLE servers ADD COLUMN geo_timezone TEXT;
