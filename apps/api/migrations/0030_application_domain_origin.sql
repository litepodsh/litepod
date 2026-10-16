ALTER TABLE application_domains ADD COLUMN is_generated INTEGER NOT NULL DEFAULT 0;

-- Preserve the origin of addresses seeded by older Litepod versions. New
-- generated domains are marked explicitly when they are created.
UPDATE application_domains
SET is_generated = 1
WHERE host = 'localhost'
   OR (host LIKE 'litepod-%.sslip.io' AND https = 0);
