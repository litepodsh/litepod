-- Enterprise identity (docs/superpowers/specs/2026-10-04-enterprise-sso-design.md).
-- Self-hosted only: the routes using these tables are not registered on cloud.
-- With no rows here the instance behaves exactly as before.

CREATE TABLE sso_connections (
    id                      TEXT PRIMARY KEY,
    organization_id         TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    kind                    TEXT NOT NULL CHECK (kind IN ('oidc', 'saml')),
    name                    TEXT NOT NULL,
    enabled                 INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    -- OIDC
    issuer                  TEXT,
    client_id               TEXT,
    client_secret_encrypted TEXT,           -- utils::security::encrypt_data
    groups_claim            TEXT NOT NULL DEFAULT 'groups',
    -- SAML (phase 7)
    idp_metadata_xml        TEXT,
    idp_entity_id           TEXT,
    -- provisioning
    jit_enabled             INTEGER NOT NULL DEFAULT 1 CHECK (jit_enabled IN (0, 1)),
    -- Base role for users who join through this connection; IdP groups add the rest.
    default_base_role_id    INTEGER NOT NULL DEFAULT 4 CHECK (default_base_role_id BETWEEN 1 AND 4),
    created_at              TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_sso_connections_org ON sso_connections(organization_id);

-- One organization per domain, instance-wide: without it, any org could claim
-- @company.com and capture its logins.
CREATE TABLE sso_domains (
    id              TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    domain          TEXT NOT NULL UNIQUE,
    verify_token    TEXT NOT NULL,
    verified_at     TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE user_identities (
    id            TEXT PRIMARY KEY,
    user_id       TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    connection_id TEXT NOT NULL REFERENCES sso_connections(id) ON DELETE CASCADE,
    subject       TEXT NOT NULL,   -- OIDC `sub`; SAML persistent NameID
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (connection_id, subject)
);
CREATE INDEX idx_user_identities_user ON user_identities(user_id);

CREATE TABLE sso_group_mappings (
    connection_id  TEXT NOT NULL REFERENCES sso_connections(id) ON DELETE CASCADE,
    external_group TEXT NOT NULL,  -- Entra group objectId, not the display name
    group_id       TEXT NOT NULL REFERENCES rbac_groups(id) ON DELETE CASCADE,
    PRIMARY KEY (connection_id, external_group)
);

-- In-flight logins: single-use, 10 minutes. `id` is the digest of the `state`
-- sent to the IdP, so the database never holds a usable value.
CREATE TABLE sso_login_states (
    id            TEXT PRIMARY KEY,
    connection_id TEXT NOT NULL REFERENCES sso_connections(id) ON DELETE CASCADE,
    nonce         TEXT NOT NULL,
    pkce_verifier TEXT NOT NULL,
    expires_at    TEXT NOT NULL
);

ALTER TABLE organizations ADD COLUMN sso_enforced INTEGER NOT NULL DEFAULT 0;
-- digest_token of the organization's SCIM bearer token; NULL = SCIM off.
ALTER TABLE organizations ADD COLUMN scim_token_digest TEXT;
