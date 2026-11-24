-- Build-only servers: a `build` server only builds images for build servers
-- (0101); no application or server group ever runs on it.
ALTER TABLE servers ADD COLUMN role TEXT NOT NULL DEFAULT 'deploy' CHECK (role IN ('deploy', 'build'));

-- Backstop for every placement path (API, assistant, MCP, templates).
CREATE TRIGGER applications_not_on_build_server_insert
BEFORE INSERT ON applications
WHEN (SELECT role FROM servers WHERE id = NEW.server_id) = 'build'
BEGIN
    SELECT RAISE(ABORT, 'applications cannot run on a build server');
END;
CREATE TRIGGER applications_not_on_build_server_update
BEFORE UPDATE OF server_id ON applications
WHEN (SELECT role FROM servers WHERE id = NEW.server_id) = 'build'
BEGIN
    SELECT RAISE(ABORT, 'applications cannot run on a build server');
END;
CREATE TRIGGER server_group_members_not_build_server
BEFORE INSERT ON server_group_members
WHEN (SELECT role FROM servers WHERE id = NEW.server_id) = 'build'
BEGIN
    SELECT RAISE(ABORT, 'a build server cannot belong to a server group');
END;

-- Registry logins saved once per organization, matched by registry host when
-- a build server pushes and the deploy server pulls.
CREATE TABLE registry_credentials (
    id                 TEXT PRIMARY KEY,
    organization_id    TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    registry           TEXT NOT NULL,
    username           TEXT NOT NULL,
    password_encrypted TEXT NOT NULL,
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL,
    UNIQUE (organization_id, registry)
);

-- Replaced by `registry_credentials`; 0101 never shipped in a release.
ALTER TABLE applications DROP COLUMN build_registry_username;
ALTER TABLE applications DROP COLUMN build_registry_password_encrypted;
