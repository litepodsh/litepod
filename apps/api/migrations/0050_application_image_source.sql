-- Applications gain a fourth deploy source: pull-and-run from a registry
-- image (`deploy_type = 'image'`). SQLite cannot extend a CHECK constraint in
-- place, so `applications` is rebuilt exactly like migration 0037 did for the
-- `static -> git` default change. The new columns (`image_url` and optional
-- registry credentials for private registries) follow the same pattern as
-- `git_url` + `git_credentials_*`: the password is encrypted by the backend
-- and never stored in clear.
PRAGMA foreign_keys = OFF;

BEGIN;

-- Four triggers on other tables name `applications` in their body (not a
-- foreign key, just a `SELECT ... FROM applications` in a `WHEN` clause).
-- Preparing a later statement in this same transaction re-validates every
-- trigger against the live schema, failing eagerly enough to see `applications`
-- mid-rebuild. Drop and recreate them around the rebuild, as in 0037/0016.
DROP TRIGGER application_runtime_endpoints_require_remote_placement_insert;
DROP TRIGGER application_runtime_endpoints_require_remote_placement_update;
DROP TRIGGER application_relay_endpoints_require_private_relay_insert;
DROP TRIGGER application_relay_endpoints_require_private_relay_update;

CREATE TABLE applications_new (
    id                     TEXT PRIMARY KEY,
    environment_id         TEXT NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    name                   TEXT NOT NULL,
    description            TEXT NOT NULL DEFAULT '',
    server_id              TEXT REFERENCES servers(id),
    deploy_type            TEXT NOT NULL DEFAULT 'git' CHECK (deploy_type IN ('static', 'git', 'compose', 'image')),
    build_method           TEXT NOT NULL DEFAULT 'docker' CHECK (build_method IN ('docker', 'compose', 'railpack')),
    -- Git deploys
    git_provider_id        TEXT REFERENCES git_providers(id) ON DELETE SET NULL,
    git_url                TEXT,
    git_branch              TEXT,
    git_repo_full_name      TEXT,
    dockerfile_path         TEXT,
    container_port           INTEGER,
    auto_deploy               INTEGER NOT NULL DEFAULT 0 CHECK (auto_deploy IN (0, 1)),
    deploy_token              TEXT,
    -- Compose deploys
    compose_file_path         TEXT,
    compose_public_service    TEXT,
    -- Railpack deploys
    railpack_project_path     TEXT NOT NULL DEFAULT '.',
    railpack_start_command    TEXT,
    -- Registry-image deploys (0050)
    image_url                 TEXT,
    registry_username         TEXT,
    registry_password_encrypted TEXT,
    -- Embedded-URL git credentials (0009); watch-path scoping (0010);
    -- uploaded icon (0012).
    git_credentials_username             TEXT,
    git_credentials_password_encrypted   TEXT,
    watch_paths                          TEXT,
    icon_url                             TEXT,
    -- Build/runtime resource overrides
    build_cpu_cores           REAL,
    build_memory_mib          INTEGER,
    build_swap_mib            INTEGER,
    clean_build               INTEGER NOT NULL DEFAULT 1,
    runtime_cpu_cores         REAL,
    runtime_memory_mib        INTEGER,
    -- Catalog template origin
    template_kind             TEXT,
    catalog_template_id       TEXT,
    catalog_icon_url          TEXT,
    catalog_primary_port      INTEGER,
    -- Boot restore (0039)
    boot_policy               TEXT NOT NULL DEFAULT 'restore' CHECK (boot_policy IN ('restore', 'always', 'notify')),
    created_at                INTEGER NOT NULL,
    updated_at                INTEGER NOT NULL,
    UNIQUE (environment_id, name)
);

INSERT INTO applications_new (
    id, environment_id, name, description, server_id, deploy_type, build_method,
    git_provider_id, git_url, git_branch, git_repo_full_name, dockerfile_path,
    container_port, auto_deploy, deploy_token, compose_file_path, compose_public_service,
    railpack_project_path, railpack_start_command, image_url, registry_username,
    registry_password_encrypted,
    git_credentials_username, git_credentials_password_encrypted, watch_paths, icon_url,
    build_cpu_cores, build_memory_mib, build_swap_mib, clean_build,
    runtime_cpu_cores, runtime_memory_mib, template_kind, catalog_template_id,
    catalog_icon_url, catalog_primary_port, boot_policy, created_at, updated_at
)
SELECT
    id, environment_id, name, description, server_id, deploy_type, build_method,
    git_provider_id, git_url, git_branch, git_repo_full_name, dockerfile_path,
    container_port, auto_deploy, deploy_token, compose_file_path, compose_public_service,
    railpack_project_path, railpack_start_command, NULL, NULL,
    NULL,
    git_credentials_username, git_credentials_password_encrypted, watch_paths, icon_url,
    build_cpu_cores, build_memory_mib, build_swap_mib, clean_build,
    runtime_cpu_cores, runtime_memory_mib, template_kind, catalog_template_id,
    catalog_icon_url, catalog_primary_port, boot_policy, created_at, updated_at
FROM applications;

DROP TABLE applications;

ALTER TABLE applications_new RENAME TO applications;

CREATE INDEX idx_applications_environment_id ON applications(environment_id);
CREATE UNIQUE INDEX idx_applications_deploy_token
    ON applications(deploy_token) WHERE deploy_token IS NOT NULL;
CREATE INDEX idx_applications_auto_deploy
    ON applications(git_provider_id, git_repo_full_name, git_branch)
    WHERE auto_deploy = 1;
CREATE INDEX idx_applications_template_kind
    ON applications(template_kind) WHERE template_kind IS NOT NULL;
CREATE INDEX idx_applications_catalog_template_id
    ON applications(catalog_template_id) WHERE catalog_template_id IS NOT NULL;
CREATE INDEX idx_applications_server_id ON applications(server_id);
CREATE TRIGGER applications_require_server_id
BEFORE INSERT ON applications
WHEN NEW.server_id IS NULL
BEGIN
    SELECT RAISE(ABORT, 'applications.server_id is required');
END;

CREATE TRIGGER application_runtime_endpoints_require_remote_placement_insert
BEFORE INSERT ON application_runtime_endpoints
WHEN NOT EXISTS (
    SELECT 1 FROM applications a JOIN servers s ON s.id = a.server_id
    WHERE a.id = NEW.application_id AND a.server_id = NEW.server_id AND s.kind = 'ssh'
)
BEGIN
    SELECT RAISE(ABORT, 'runtime endpoint must match a remote application placement');
END;
CREATE TRIGGER application_runtime_endpoints_require_remote_placement_update
BEFORE UPDATE OF application_id, server_id ON application_runtime_endpoints
WHEN NOT EXISTS (
    SELECT 1 FROM applications a JOIN servers s ON s.id = a.server_id
    WHERE a.id = NEW.application_id AND a.server_id = NEW.server_id AND s.kind = 'ssh'
)
BEGIN
    SELECT RAISE(ABORT, 'runtime endpoint must match a remote application placement');
END;
CREATE TRIGGER application_relay_endpoints_require_private_relay_insert
BEFORE INSERT ON application_relay_endpoints
WHEN NOT EXISTS (
    SELECT 1 FROM applications a JOIN servers s ON s.id = a.server_id
    WHERE a.id = NEW.application_id AND a.server_id = NEW.server_id AND s.kind = 'ssh' AND s.ingress_mode = 'private_relay'
)
BEGIN
    SELECT RAISE(ABORT, 'relay endpoint must match a private-relay application placement');
END;
CREATE TRIGGER application_relay_endpoints_require_private_relay_update
BEFORE UPDATE OF application_id, server_id ON application_relay_endpoints
WHEN NOT EXISTS (
    SELECT 1 FROM applications a JOIN servers s ON s.id = a.server_id
    WHERE a.id = NEW.application_id AND a.server_id = NEW.server_id AND s.kind = 'ssh' AND s.ingress_mode = 'private_relay'
)
BEGIN
    SELECT RAISE(ABORT, 'relay endpoint must match a private-relay application placement');
END;

COMMIT;

PRAGMA foreign_keys = ON;