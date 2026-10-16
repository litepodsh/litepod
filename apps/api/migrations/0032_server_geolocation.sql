-- Where each server physically is, resolved from its public address by ip.guide.
-- The columns are the cache: a lookup survives a restart and is only refreshed
-- when `geo_resolved_at` goes stale.
-- SQLite rejects UNIQUE inside ALTER TABLE ADD COLUMN, and none of these need it.
ALTER TABLE servers ADD COLUMN latitude REAL;
ALTER TABLE servers ADD COLUMN longitude REAL;
ALTER TABLE servers ADD COLUMN geo_city TEXT;
ALTER TABLE servers ADD COLUMN geo_country TEXT;
ALTER TABLE servers ADD COLUMN geo_asn TEXT;
-- Epoch millis of the last attempt, successful or not, so a permanently
-- unresolvable address is not looked up on every page load.
ALTER TABLE servers ADD COLUMN geo_resolved_at INTEGER;
