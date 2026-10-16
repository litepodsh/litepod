-- Which connection serves a domain. NULL keeps the old rule (the org's oldest
-- enabled connection), so existing setups behave the same.
ALTER TABLE sso_domains ADD COLUMN connection_id TEXT REFERENCES sso_connections(id) ON DELETE SET NULL;
