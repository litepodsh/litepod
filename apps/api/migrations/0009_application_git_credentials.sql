-- Credentials embedded directly in a pasted git URL (e.g.
-- https://user:token@github.com/owner/repo.git), extracted and stripped at
-- save time so `applications.git_url` never contains a secret. Mirrors
-- git_providers.password_encrypted (AES-256-GCM via utils/security.rs).
-- NULL/NULL means: no saved provider AND no embedded credentials (anonymous
-- public clone) or a git_provider_id is set (the provider takes precedence
-- and these columns are cleared).
ALTER TABLE applications ADD COLUMN git_credentials_username TEXT;
ALTER TABLE applications ADD COLUMN git_credentials_password_encrypted TEXT;
