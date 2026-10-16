-- Every [[ports]] entry of the catalog template an application came from, as
-- JSON [{"container":8080,"service":null,"primary":false}], so `{{url:8080}}`
-- env markers resolve without refetching the catalog. NULL for applications
-- created before this column; they fall back to catalog_primary_port.
ALTER TABLE applications ADD COLUMN catalog_ports TEXT;
ALTER TABLE applications ADD COLUMN catalog_requires_https INTEGER NOT NULL DEFAULT 0;
ALTER TABLE applications ADD COLUMN catalog_url_persisted INTEGER NOT NULL DEFAULT 0;
