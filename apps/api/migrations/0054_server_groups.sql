-- Server groups: an organization-scoped collection of remote servers an
-- application can target instead of a single server. Targeting a group fans
-- the app out as one independent replica per member. Groups are remote-only
-- (`kind = 'ssh'`); the local server can never be a member. This is enforced
-- in the repository layer, not here, because SQLite CHECK constraints cannot
-- reach across a foreign key cleanly.

CREATE TABLE server_groups (
    id              TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    UNIQUE (organization_id, name)
);
CREATE INDEX idx_server_groups_organization_id ON server_groups(organization_id);

CREATE TABLE server_group_members (
    group_id   TEXT NOT NULL REFERENCES server_groups(id) ON DELETE CASCADE,
    server_id  TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (group_id, server_id)
);
CREATE INDEX idx_server_group_members_server_id ON server_group_members(server_id);

-- Applications may target a group instead of a single server. Added as a
-- plain column (no rebuild) because SQLite allows `ADD COLUMN ... REFERENCES`
-- when the default is NULL. Must be created after `server_groups`.
ALTER TABLE applications ADD COLUMN server_group_id TEXT REFERENCES server_groups(id);
CREATE INDEX idx_applications_server_group_id ON applications(server_group_id);

-- Replace the old insert-only "server_id is required" trigger with an
-- exactly-one-of (server_id XOR server_group_id) rule, for both INSERT and
-- UPDATE of the target columns.
DROP TRIGGER applications_require_server_id;
CREATE TRIGGER applications_require_target_insert
BEFORE INSERT ON applications
WHEN (NEW.server_id IS NULL AND NEW.server_group_id IS NULL)
  OR (NEW.server_id IS NOT NULL AND NEW.server_group_id IS NOT NULL)
BEGIN
    SELECT RAISE(ABORT, 'applications require exactly one of server_id or server_group_id');
END;
CREATE TRIGGER applications_require_target_update
BEFORE UPDATE OF server_id, server_group_id ON applications
WHEN (NEW.server_id IS NULL AND NEW.server_group_id IS NULL)
  OR (NEW.server_id IS NOT NULL AND NEW.server_group_id IS NOT NULL)
BEGIN
    SELECT RAISE(ABORT, 'applications require exactly one of server_id or server_group_id');
END;

-- Per-server deployment state for group-targeted apps.
CREATE TABLE application_replicas (
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    server_id      TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    status         TEXT NOT NULL DEFAULT 'not_deployed'
        CHECK (status IN ('not_deployed', 'deploying', 'running', 'failed', 'stopped')),
    container_name TEXT NOT NULL DEFAULT '',
    last_error     TEXT,
    deployed_at    INTEGER,
    updated_at     INTEGER NOT NULL,
    PRIMARY KEY (application_id, server_id)
);
CREATE INDEX idx_application_replicas_server_id ON application_replicas(server_id);

-- Per-domain pin: which server a domain routes to. NULL for single-server
-- apps (derived from the application); required for group apps.
ALTER TABLE application_domains ADD COLUMN server_id TEXT REFERENCES servers(id);

-- Widen runtime endpoints so a group app gets one loopback endpoint per
-- replica. Current PK is (application_id, container_port); add server_id.
DROP TRIGGER application_runtime_endpoints_require_remote_placement_insert;
DROP TRIGGER application_runtime_endpoints_require_remote_placement_update;

ALTER TABLE application_runtime_endpoints RENAME TO application_runtime_endpoints_v1;

CREATE TABLE application_runtime_endpoints (
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    server_id      TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    host_port      INTEGER NOT NULL CHECK (host_port BETWEEN 1024 AND 65535),
    container_port INTEGER NOT NULL CHECK (container_port BETWEEN 1 AND 65535),
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    PRIMARY KEY (application_id, server_id, container_port),
    UNIQUE (server_id, host_port)
);

INSERT INTO application_runtime_endpoints
    (application_id, server_id, host_port, container_port, created_at, updated_at)
SELECT application_id, server_id, host_port, container_port, created_at, updated_at
FROM application_runtime_endpoints_v1;

DROP TABLE application_runtime_endpoints_v1;

CREATE INDEX idx_application_runtime_endpoints_server
    ON application_runtime_endpoints(server_id, host_port);

CREATE TRIGGER application_runtime_endpoints_require_remote_placement_insert
BEFORE INSERT ON application_runtime_endpoints
WHEN NOT EXISTS (
    SELECT 1 FROM applications a JOIN servers s ON s.id = NEW.server_id
    WHERE a.id = NEW.application_id
      AND (
        a.server_id = NEW.server_id
        OR EXISTS (SELECT 1 FROM server_group_members m
                   WHERE m.group_id = a.server_group_id AND m.server_id = NEW.server_id)
      )
      AND s.kind = 'ssh'
)
BEGIN
    SELECT RAISE(ABORT, 'runtime endpoint must match a remote application placement');
END;
CREATE TRIGGER application_runtime_endpoints_require_remote_placement_update
BEFORE UPDATE OF application_id, server_id ON application_runtime_endpoints
WHEN NOT EXISTS (
    SELECT 1 FROM applications a JOIN servers s ON s.id = NEW.server_id
    WHERE a.id = NEW.application_id
      AND (
        a.server_id = NEW.server_id
        OR EXISTS (SELECT 1 FROM server_group_members m
                   WHERE m.group_id = a.server_group_id AND m.server_id = NEW.server_id)
      )
      AND s.kind = 'ssh'
)
BEGIN
    SELECT RAISE(ABORT, 'runtime endpoint must match a remote application placement');
END;

-- Same widening for relay endpoints (PrivateRelay ingress).
DROP TRIGGER application_relay_endpoints_require_private_relay_insert;
DROP TRIGGER application_relay_endpoints_require_private_relay_update;

ALTER TABLE application_relay_endpoints RENAME TO application_relay_endpoints_v1;

CREATE TABLE application_relay_endpoints (
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    server_id      TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    relay_port     INTEGER NOT NULL CHECK (relay_port BETWEEN 43000 AND 43999),
    container_port INTEGER NOT NULL CHECK (container_port BETWEEN 1 AND 65535),
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    PRIMARY KEY (application_id, server_id, container_port),
    UNIQUE (relay_port)
);

INSERT INTO application_relay_endpoints
    (application_id, server_id, relay_port, container_port, created_at, updated_at)
SELECT application_id, server_id, relay_port, container_port, created_at, updated_at
FROM application_relay_endpoints_v1;

DROP TABLE application_relay_endpoints_v1;

CREATE TRIGGER application_relay_endpoints_require_private_relay_insert
BEFORE INSERT ON application_relay_endpoints
WHEN NOT EXISTS (
    SELECT 1 FROM applications a JOIN servers s ON s.id = NEW.server_id
    WHERE a.id = NEW.application_id
      AND (
        a.server_id = NEW.server_id
        OR EXISTS (SELECT 1 FROM server_group_members m
                   WHERE m.group_id = a.server_group_id AND m.server_id = NEW.server_id)
      )
      AND s.kind = 'ssh' AND s.ingress_mode = 'private_relay'
)
BEGIN
    SELECT RAISE(ABORT, 'relay endpoint must match a private-relay application placement');
END;
CREATE TRIGGER application_relay_endpoints_require_private_relay_update
BEFORE UPDATE OF application_id, server_id ON application_relay_endpoints
WHEN NOT EXISTS (
    SELECT 1 FROM applications a JOIN servers s ON s.id = NEW.server_id
    WHERE a.id = NEW.application_id
      AND (
        a.server_id = NEW.server_id
        OR EXISTS (SELECT 1 FROM server_group_members m
                   WHERE m.group_id = a.server_group_id AND m.server_id = NEW.server_id)
      )
      AND s.kind = 'ssh' AND s.ingress_mode = 'private_relay'
)
BEGIN
    SELECT RAISE(ABORT, 'relay endpoint must match a private-relay application placement');
END;
