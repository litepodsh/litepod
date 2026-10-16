-- Seed data for the genesis schema (0001_init.sql). Kept separate from the
-- schema so table/index/trigger definitions can be read on their own. Order
-- follows FK dependency (status_type before status; everything else here is
-- either standalone or nullable-FK, so order among the rest doesn't matter).

INSERT INTO status_type (status_type_id, name, description) VALUES
    (1, 'Users', 'Status type for Users');

INSERT INTO status (status_id, name, description, status_type_id) VALUES
    (1, 'Active', 'User is active', 1),
    (2, 'Inactive', 'User is inactive', 1),
    (3, 'Pending', 'User is pending', 1);

INSERT INTO roles (id, name, description) VALUES
    (0, 'SysAdmin', 'Full system access, all permissions'),
    (1, 'Admin', 'Manage organization, members, full CRUD'),
    (2, 'Editor', 'CRUD projects and environments'),
    (3, 'Viewer', 'Read-only access');

INSERT INTO build_configuration (id, max_concurrent_builds) VALUES (1, 3);

INSERT INTO rate_limit_settings (id) VALUES (1);

-- The local server row every install ships with (kind = 'local', id = 'local'
-- is enforced by the servers CHECK constraint in 0001_init.sql).
INSERT INTO servers (id, kind, name, ingress_mode, state, podman_mode, created_at, updated_at)
    VALUES ('local', 'local', 'Local server', 'central', 'ready', 'rootless', unixepoch() * 1000, unixepoch() * 1000);
