-- One row per application that has touched basic-auth settings. The row is
-- created lazily on the first PUT (or first user add) so the same id is
-- stable across edits.
CREATE TABLE application_basic_auth (
    id              TEXT PRIMARY KEY,
    application_id  TEXT NOT NULL UNIQUE REFERENCES applications(id) ON DELETE CASCADE,
    enabled         INTEGER NOT NULL DEFAULT 0,
    realm           TEXT NOT NULL DEFAULT 'Restricted',
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

-- Caddy `basicauth` requires one bcrypt-hashed password per account. The hash
-- is never returned via the API; `has_password` is the only signal the UI has.
CREATE TABLE application_basic_auth_users (
    id              TEXT PRIMARY KEY,
    auth_id         TEXT NOT NULL REFERENCES application_basic_auth(id) ON DELETE CASCADE,
    username        TEXT NOT NULL,
    password_hash   TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE UNIQUE INDEX idx_basic_auth_users_username ON application_basic_auth_users(auth_id, username);

-- Selected-domain table. A row here means this domain's Caddy route gets the
-- basicauth handler prepended; absence means the route is unprotected.
CREATE TABLE application_basic_auth_domains (
    auth_id     TEXT NOT NULL REFERENCES application_basic_auth(id) ON DELETE CASCADE,
    domain_id   TEXT NOT NULL REFERENCES application_domains(id) ON DELETE CASCADE,
    PRIMARY KEY (auth_id, domain_id)
);
CREATE INDEX idx_basic_auth_domains_domain ON application_basic_auth_domains(domain_id);
