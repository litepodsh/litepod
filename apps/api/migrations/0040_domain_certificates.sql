CREATE TABLE domain_certificates (
    route_id TEXT PRIMARY KEY,
    provider TEXT NOT NULL DEFAULT 'automatic' CHECK(provider IN ('automatic', 'letsencrypt', 'custom')),
    certificate_pem TEXT,
    encrypted_private_key TEXT
);

CREATE TRIGGER delete_domain_certificate AFTER DELETE ON application_domains
BEGIN
    DELETE FROM domain_certificates WHERE route_id = 'litepod-route-' || OLD.id;
END;
