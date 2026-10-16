-- Granular RBAC (docs/superpowers/specs/2026-10-04-rbac-permissions-design.md).
--
-- organization_members.role_id stays the org-wide base role. The tables below
-- only ADD grants on top of it. System roles' permissions live in code
-- (security/authz/permission.rs), so they are referenced by roles.id and have
-- no rbac_role_permissions rows; only custom roles do.

-- Base role for "member of the org, access only through assignments".
INSERT INTO roles (id, name, description) VALUES
    (4, 'Member', 'Organization membership only; access comes from scoped assignments');

CREATE TABLE rbac_roles (
    id              TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (organization_id, name)
);

CREATE TABLE rbac_role_permissions (
    role_id    TEXT NOT NULL REFERENCES rbac_roles(id) ON DELETE CASCADE,
    permission TEXT NOT NULL,
    PRIMARY KEY (role_id, permission)
);

CREATE TABLE rbac_groups (
    id              TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    source          TEXT NOT NULL DEFAULT 'local' CHECK (source IN ('local', 'idp', 'scim')),
    external_id     TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (organization_id, name)
);
CREATE UNIQUE INDEX idx_rbac_groups_external
    ON rbac_groups(organization_id, external_id) WHERE external_id IS NOT NULL;

CREATE TABLE rbac_group_members (
    group_id TEXT NOT NULL REFERENCES rbac_groups(id) ON DELETE CASCADE,
    user_id  TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (group_id, user_id)
);
CREATE INDEX idx_rbac_group_members_user ON rbac_group_members(user_id);

-- principal_id and scope_id are polymorphic (no FK); the triggers below keep
-- them from dangling.
CREATE TABLE rbac_assignments (
    id              TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    principal_type  TEXT NOT NULL CHECK (principal_type IN ('user', 'group')),
    principal_id    TEXT NOT NULL,
    system_role_id  INTEGER REFERENCES roles(id),
    custom_role_id  TEXT REFERENCES rbac_roles(id) ON DELETE CASCADE,
    scope_type      TEXT NOT NULL CHECK (scope_type IN ('org', 'project', 'environment')),
    scope_id        TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK ((system_role_id IS NULL) <> (custom_role_id IS NULL)),
    CHECK (system_role_id IS NULL OR system_role_id IN (1, 2, 3))
);
CREATE UNIQUE INDEX idx_rbac_assignments_unique ON rbac_assignments(
    principal_type, principal_id, COALESCE(system_role_id, -1), COALESCE(custom_role_id, ''),
    scope_type, scope_id
);
CREATE INDEX idx_rbac_assignments_org_principal
    ON rbac_assignments(organization_id, principal_type, principal_id);

CREATE TRIGGER rbac_assignments_on_user_delete AFTER DELETE ON users
BEGIN
    DELETE FROM rbac_assignments WHERE principal_type = 'user' AND principal_id = OLD.id;
END;

CREATE TRIGGER rbac_assignments_on_group_delete AFTER DELETE ON rbac_groups
BEGIN
    DELETE FROM rbac_assignments WHERE principal_type = 'group' AND principal_id = OLD.id;
END;

CREATE TRIGGER rbac_assignments_on_project_delete AFTER DELETE ON projects
BEGIN
    DELETE FROM rbac_assignments WHERE scope_type = 'project' AND scope_id = OLD.id;
END;

CREATE TRIGGER rbac_assignments_on_environment_delete AFTER DELETE ON environments
BEGIN
    DELETE FROM rbac_assignments WHERE scope_type = 'environment' AND scope_id = OLD.id;
END;

CREATE TRIGGER rbac_on_member_leave AFTER DELETE ON organization_members
BEGIN
    DELETE FROM rbac_assignments
        WHERE organization_id = OLD.organization_id
          AND principal_type = 'user' AND principal_id = OLD.user_id;
    DELETE FROM rbac_group_members
        WHERE user_id = OLD.user_id
          AND group_id IN (SELECT id FROM rbac_groups WHERE organization_id = OLD.organization_id);
END;
