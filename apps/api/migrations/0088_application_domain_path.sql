ALTER TABLE application_domains ADD COLUMN path TEXT;
DROP INDEX idx_application_domains_host;
CREATE UNIQUE INDEX idx_application_domains_host ON application_domains(host, port, COALESCE(path, ''));
