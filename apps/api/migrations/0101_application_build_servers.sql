-- Dedicated build servers: a Git application can build on another server,
-- push the image to `build_image_repository`, and have its deploy server pull
-- that exact digest. NULL `build_server_id` keeps building where it runs.
ALTER TABLE applications ADD COLUMN build_server_id TEXT REFERENCES servers(id) ON DELETE SET NULL;
ALTER TABLE applications ADD COLUMN build_image_repository TEXT;
ALTER TABLE applications ADD COLUMN build_registry_username TEXT;
ALTER TABLE applications ADD COLUMN build_registry_password_encrypted TEXT;
