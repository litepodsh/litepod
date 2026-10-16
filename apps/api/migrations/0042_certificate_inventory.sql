CREATE TABLE https_certificates (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    certificate_pem TEXT NOT NULL,
    encrypted_private_key TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
ALTER TABLE domain_certificates ADD COLUMN certificate_id TEXT REFERENCES https_certificates(id) ON DELETE RESTRICT;
INSERT INTO https_certificates (id, name, certificate_pem, encrypted_private_key, created_at, updated_at)
SELECT route_id, 'Imported: ' || COALESCE((SELECT host FROM application_domains WHERE 'litepod-route-' || id = route_id), 'LitePod panel'), certificate_pem, encrypted_private_key, unixepoch() * 1000, unixepoch() * 1000
FROM domain_certificates WHERE provider = 'custom' AND certificate_pem IS NOT NULL AND encrypted_private_key IS NOT NULL;
UPDATE domain_certificates SET certificate_id = route_id, certificate_pem = NULL, encrypted_private_key = NULL WHERE route_id IN (SELECT id FROM https_certificates);
