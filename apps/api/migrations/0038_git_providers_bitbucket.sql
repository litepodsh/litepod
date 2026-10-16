-- Bitbucket Cloud joins the provider list, OAuth-only (Bitbucket removed App
-- Passwords entirely on 2026-07-28; OAuth consumer + client secret is the
-- only auth method left, same shape as the existing GitLab OAuth columns on
-- this table). SQLite cannot alter a CHECK constraint in place, so
-- `git_providers` is rebuilt with both allowlists widened.
PRAGMA foreign_keys = OFF;

BEGIN;

CREATE TABLE git_providers_new (
    id                            TEXT PRIMARY KEY,
    organization_id               TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name                          TEXT NOT NULL,
    provider_type                 TEXT NOT NULL
        CHECK (provider_type IN ('github', 'gitea', 'forgejo', 'gitlab', 'bitbucket', 'generic')),
    host_url                      TEXT NOT NULL DEFAULT '',
    username                      TEXT NOT NULL,
    password_encrypted            TEXT NOT NULL,
    auth_method                   TEXT NOT NULL DEFAULT 'token'
        CHECK (auth_method IN ('oauth2', 'token', 'gitlab_oauth', 'bitbucket_oauth')),
    kind                          TEXT NOT NULL DEFAULT 'credentials'
        CHECK (kind IN ('credentials', 'github_app')),
    created_at                    INTEGER NOT NULL,
    updated_at                    INTEGER NOT NULL,
    -- GitLab OAuth / Bitbucket OAuth (shared columns, scoped by provider_type)
    oauth_client_id               TEXT,
    oauth_client_secret_encrypted TEXT,
    oauth_redirect_uri            TEXT,
    oauth_access_token_encrypted  TEXT,
    oauth_refresh_token_encrypted TEXT,
    oauth_expires_at              INTEGER,
    oauth_scopes                  TEXT,
    oauth_user_id                 TEXT,
    oauth_user_login              TEXT,
    oauth_connected_at            INTEGER,
    -- GitLab webhooks use a shared secret in `X-Gitlab-Token`; Bitbucket
    -- webhooks HMAC-sign the body with it instead (`X-Hub-Signature`).
    webhook_secret                TEXT
);

INSERT INTO git_providers_new SELECT * FROM git_providers;

DROP TABLE git_providers;

ALTER TABLE git_providers_new RENAME TO git_providers;

CREATE UNIQUE INDEX idx_git_providers_organization_name ON git_providers(organization_id, name);

COMMIT;

PRAGMA foreign_keys = ON;
