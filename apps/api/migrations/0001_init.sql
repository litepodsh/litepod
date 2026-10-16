-- Litepod genesis schema (tables, indexes, triggers only — no data). This
-- replaces the previous 0001-0038 incremental history: it is the exact
-- cumulative schema that chain produced (verified by running the old chain
-- against a fresh DB and diffing `sqlite_master`), reorganized into logical
-- sections and with historical ALTER-TABLE column tack-ons folded back into
-- their table's column list, plus one deliberate behavior change folded in
-- directly rather than kept as a separate follow-up migration:
-- `application_runtime_endpoints.server_id` and
-- `application_relay_endpoints.server_id` are `ON DELETE CASCADE` instead of
-- `ON DELETE RESTRICT`, so removing a server cleans up its endpoint rows
-- instead of blocking the delete. Seed data lives in `0002_seed.sql`.

PRAGMA foreign_keys = ON;

-- ── Core reference data ─────────────────────────────────────────────────

CREATE TABLE status_type (
    status_type_id INTEGER PRIMARY KEY,
    name           TEXT NOT NULL UNIQUE,
    description    TEXT NOT NULL DEFAULT ''
);

CREATE TABLE status (
    status_id      INTEGER PRIMARY KEY,
    name           TEXT NOT NULL,
    description    TEXT NOT NULL DEFAULT '',
    status_type_id INTEGER NOT NULL REFERENCES status_type(status_type_id)
);
CREATE UNIQUE INDEX idx_status_name_per_type ON status(status_type_id, name);

CREATE TABLE roles (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL
);

-- ── Users, auth & RBAC ──────────────────────────────────────────────────

CREATE TABLE users (
    id                             TEXT PRIMARY KEY,
    email                          TEXT NOT NULL UNIQUE,
    hashed_password                TEXT NOT NULL,
    name                           TEXT NOT NULL,
    status_id                      INTEGER NOT NULL DEFAULT 1 REFERENCES status(status_id),
    totp_secret                    TEXT,
    totp_enabled                   INTEGER DEFAULT 0,
    email_verification_expires_at  TEXT,
    deactivated_at                 TEXT,
    created_at                     TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE UNIQUE INDEX idx_users_name ON users(name);
CREATE INDEX idx_users_pending_email_verification_expiry
    ON users(email_verification_expires_at)
    WHERE status_id = 3;

CREATE TABLE sessions (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token       TEXT NOT NULL UNIQUE,
    expires_at  TEXT NOT NULL,
    user_agent  TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_sessions_user_id ON sessions(user_id);
CREATE INDEX idx_sessions_token   ON sessions(token);

-- A single global SysAdmin (role_id = 0) is enforced by the partial unique
-- index below, not by application code alone.
CREATE TABLE user_global_roles (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role_id INTEGER NOT NULL REFERENCES roles(id),
    PRIMARY KEY (user_id, role_id)
);
CREATE UNIQUE INDEX idx_user_global_roles_single_sysadmin
    ON user_global_roles(role_id)
    WHERE role_id = 0;

CREATE TABLE recovery_codes (
    id             TEXT PRIMARY KEY,
    user_id        TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    encrypted_code TEXT NOT NULL,
    used           INTEGER DEFAULT 0,
    created_at     TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_recovery_codes_user_id ON recovery_codes(user_id);

-- One pending invitation per user; the token is only ever compared as a hash.
CREATE TABLE invitations (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
    created_by  TEXT NOT NULL REFERENCES users(id),
    token_hash  TEXT NOT NULL UNIQUE,
    expires_at  TEXT NOT NULL,
    accepted_at TEXT,
    revoked_at  TEXT,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_invitations_pending
    ON invitations(expires_at)
    WHERE accepted_at IS NULL AND revoked_at IS NULL;
CREATE INDEX idx_invitations_created_by ON invitations(created_by);

-- ── Organizations ───────────────────────────────────────────────────────

CREATE TABLE organizations (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    slug        TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    icon        TEXT NOT NULL DEFAULT '',
    icon_bg     TEXT NOT NULL DEFAULT '',
    plan        TEXT NOT NULL DEFAULT 'System',
    is_root     INTEGER NOT NULL DEFAULT 0,
    created_by  TEXT NOT NULL REFERENCES users(id),
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX idx_organizations_created_by ON organizations(created_by);

CREATE TABLE organization_members (
    organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role_id         INTEGER NOT NULL REFERENCES roles(id),
    joined_at       TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (organization_id, user_id)
);
CREATE INDEX idx_organization_members_user_id ON organization_members(user_id);

-- ── Servers ─────────────────────────────────────────────────────────────

CREATE TABLE servers (
    id                     TEXT PRIMARY KEY,
    kind                   TEXT NOT NULL CHECK (kind IN ('local', 'ssh')),
    name                   TEXT NOT NULL UNIQUE,
    organization_id        TEXT REFERENCES organizations(id) ON DELETE CASCADE,
    ingress_mode           TEXT NOT NULL CHECK (ingress_mode IN ('central', 'public_edge', 'private_relay', 'private_only')),
    state                  TEXT NOT NULL CHECK (state IN ('needs_verification', 'needs_setup', 'setting_up', 'ready', 'degraded', 'offline', 'unsupported')),
    podman_mode            TEXT NOT NULL DEFAULT 'rootless',
    public_address         TEXT,
    -- SSH connection details (remote servers only)
    ssh_host               TEXT,
    ssh_port               INTEGER,
    ssh_user               TEXT,
    host_key_fingerprint   TEXT,
    host_key_known_hosts   TEXT,
    private_key_encrypted  TEXT,
    facts_json             TEXT NOT NULL DEFAULT '{}',
    last_checked_at        INTEGER,
    last_error_code        TEXT,
    last_error_message     TEXT,
    created_at             INTEGER NOT NULL,
    updated_at             INTEGER NOT NULL,
    CHECK (
        (kind = 'local' AND id = 'local' AND ingress_mode = 'central')
        OR
        (kind = 'ssh' AND ssh_host IS NOT NULL AND ssh_port BETWEEN 1 AND 65535 AND ssh_user IS NOT NULL)
    )
);
CREATE INDEX idx_servers_organization ON servers(organization_id);

CREATE TABLE server_jobs (
    id             TEXT PRIMARY KEY,
    server_id      TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    kind           TEXT NOT NULL CHECK (kind IN ('preflight', 'setup', 'repair', 'reconcile_ingress', 'rotate_key')),
    status         TEXT NOT NULL CHECK (status IN ('queued', 'running', 'succeeded', 'failed')),
    actor_user_id  TEXT NOT NULL REFERENCES users(id),
    phase          TEXT NOT NULL,
    log_redacted   TEXT NOT NULL DEFAULT '',
    error_code     TEXT,
    error_message  TEXT,
    started_at     INTEGER,
    finished_at    INTEGER,
    created_at     INTEGER NOT NULL
);
CREATE INDEX idx_server_jobs_server_created ON server_jobs(server_id, created_at DESC);
-- Only one Setup or Repair job may be in flight per server at a time.
CREATE UNIQUE INDEX idx_server_one_active_setup_job
    ON server_jobs(server_id)
    WHERE kind IN ('setup', 'repair') AND status IN ('queued', 'running');

-- ── Projects, environments, applications ───────────────────────────────

CREATE TABLE projects (
    id              TEXT PRIMARY KEY,
    organization_id TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    UNIQUE (organization_id, name)
);
CREATE INDEX idx_projects_organization_id ON projects(organization_id);

CREATE TABLE environments (
    id            TEXT PRIMARY KEY,
    project_id    TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    is_production INTEGER NOT NULL DEFAULT 0 CHECK (is_production IN (0, 1)),
    description   TEXT NOT NULL DEFAULT '',
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    UNIQUE (project_id, name)
);
CREATE INDEX idx_environments_project_id ON environments(project_id);
CREATE UNIQUE INDEX idx_single_production_environment
    ON environments(project_id) WHERE is_production = 1;

CREATE TABLE applications (
    id                     TEXT PRIMARY KEY,
    environment_id         TEXT NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    name                   TEXT NOT NULL,
    description            TEXT NOT NULL DEFAULT '',
    -- `server_id` is nullable here only so historical rows can exist; every
    -- new row is required to set it by the `applications_require_server_id`
    -- trigger below (deliberately not a NOT NULL constraint — see trigger).
    server_id              TEXT REFERENCES servers(id),
    deploy_type            TEXT NOT NULL DEFAULT 'static' CHECK (deploy_type IN ('static', 'git', 'compose')),
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
    created_at                INTEGER NOT NULL,
    updated_at                INTEGER NOT NULL,
    UNIQUE (environment_id, name)
);
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

CREATE TABLE application_artifacts (
    id             TEXT PRIMARY KEY,
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    filename       TEXT NOT NULL,
    kind           TEXT NOT NULL CHECK (kind IN ('zip', 'html')),
    byte_size      INTEGER NOT NULL,
    sha256         TEXT NOT NULL,
    relative_path  TEXT NOT NULL,
    created_at     INTEGER NOT NULL
);
CREATE INDEX idx_application_artifacts_application_id
    ON application_artifacts(application_id, created_at DESC);

CREATE TABLE application_deployments (
    application_id TEXT PRIMARY KEY REFERENCES applications(id) ON DELETE CASCADE,
    artifact_id    TEXT REFERENCES application_artifacts(id) ON DELETE SET NULL,
    status         TEXT NOT NULL DEFAULT 'not_deployed'
        CHECK (status IN ('not_deployed', 'ready', 'deploying', 'running', 'failed', 'stopped')),
    host_port      INTEGER UNIQUE,
    container_name TEXT NOT NULL,
    hostname       TEXT NOT NULL,
    config_stale   INTEGER NOT NULL DEFAULT 0,
    last_error     TEXT,
    deployed_at    INTEGER,
    updated_at     INTEGER NOT NULL
);

CREATE TABLE application_environment_variables (
    application_id  TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    scope           TEXT NOT NULL CHECK (scope IN ('runtime', 'build')),
    name            TEXT NOT NULL,
    encrypted_value TEXT NOT NULL,
    updated_at      INTEGER NOT NULL,
    PRIMARY KEY (application_id, scope, name)
);
CREATE INDEX idx_application_environment_variables_scope
    ON application_environment_variables(application_id, scope, name);

CREATE TABLE application_domains (
    id             TEXT PRIMARY KEY,
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    host           TEXT NOT NULL,
    port           INTEGER NOT NULL,
    https          INTEGER NOT NULL DEFAULT 0,
    is_primary     INTEGER NOT NULL DEFAULT 0,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);
CREATE UNIQUE INDEX idx_application_domains_host ON application_domains(host, port);
CREATE UNIQUE INDEX idx_application_domains_primary
    ON application_domains(application_id) WHERE is_primary = 1;
CREATE INDEX idx_application_domains_application_id
    ON application_domains(application_id, created_at);

CREATE TABLE application_runtime_endpoints (
    application_id TEXT PRIMARY KEY REFERENCES applications(id) ON DELETE CASCADE,
    server_id      TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    host_port      INTEGER NOT NULL CHECK (host_port BETWEEN 42000 AND 42999),
    container_port INTEGER NOT NULL CHECK (container_port BETWEEN 1 AND 65535),
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    UNIQUE (server_id, host_port)
);
CREATE INDEX idx_application_runtime_endpoints_server
    ON application_runtime_endpoints(server_id, host_port);
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

CREATE TABLE application_relay_endpoints (
    application_id TEXT PRIMARY KEY REFERENCES applications(id) ON DELETE CASCADE,
    server_id      TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    relay_port     INTEGER NOT NULL CHECK (relay_port BETWEEN 43000 AND 43999),
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    UNIQUE (relay_port)
);
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

CREATE TABLE application_compose_proxy_routes (
    domain_id      TEXT PRIMARY KEY REFERENCES application_domains(id) ON DELETE CASCADE,
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    service        TEXT NOT NULL,
    container_port INTEGER NOT NULL CHECK (container_port BETWEEN 1 AND 65535),
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);
CREATE INDEX idx_compose_proxy_routes_application
    ON application_compose_proxy_routes(application_id, domain_id);

CREATE TABLE application_managed_volumes (
    id              TEXT PRIMARY KEY,
    application_id  TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    podman_name     TEXT NOT NULL UNIQUE,
    mount_path      TEXT NOT NULL,
    compose_service TEXT,
    attached        INTEGER NOT NULL DEFAULT 1 CHECK (attached IN (0, 1)),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    UNIQUE (application_id, name)
);
CREATE INDEX idx_application_managed_volumes_application
    ON application_managed_volumes(application_id, attached, created_at);

-- ── Deployment runs & the build queue ──────────────────────────────────

CREATE TABLE application_deployment_runs (
    id                   TEXT PRIMARY KEY,
    application_id       TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    source_type          TEXT NOT NULL,
    build_method         TEXT NOT NULL,
    status               TEXT NOT NULL CHECK (status IN ('building', 'deploying', 'succeeded', 'failed', 'canceled')),
    started_at           INTEGER NOT NULL,
    finished_at          INTEGER,
    last_error           TEXT,
    commit_sha           TEXT,
    commit_message       TEXT,
    commit_author_name   TEXT,
    commit_author_email  TEXT,
    triggered_by         TEXT CHECK (triggered_by IN ('manual', 'manual_deploy', 'manual_start', 'manual_stop', 'manual_remove', 'webhook', 'deploy_hook')),
    branch               TEXT,
    created_by           TEXT REFERENCES users(id) ON DELETE SET NULL
);
CREATE INDEX idx_deployment_runs_application_started
    ON application_deployment_runs(application_id, started_at DESC);
CREATE INDEX idx_deployment_runs_created_by ON application_deployment_runs(created_by);

CREATE TABLE application_deployment_run_logs (
    id         TEXT PRIMARY KEY,
    run_id     TEXT NOT NULL REFERENCES application_deployment_runs(id) ON DELETE CASCADE,
    sequence   INTEGER NOT NULL,
    level      TEXT NOT NULL CHECK (level IN ('info', 'error')),
    message    TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (run_id, sequence)
);
CREATE INDEX idx_deployment_run_logs_run_sequence
    ON application_deployment_run_logs(run_id, sequence);

CREATE TABLE deployment_jobs (
    id                      TEXT PRIMARY KEY,
    application_id          TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    project_id              TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    environment_id          TEXT NOT NULL REFERENCES environments(id) ON DELETE CASCADE,
    organization_id         TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    run_id                  TEXT NOT NULL REFERENCES application_deployment_runs(id) ON DELETE CASCADE,
    source_type             TEXT NOT NULL,
    source_snapshot         TEXT NOT NULL,
    build_method            TEXT NOT NULL,
    build_snapshot          TEXT NOT NULL,
    status                  TEXT NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'building', 'deploying', 'succeeded', 'failed', 'canceled')),
    queued_at               INTEGER NOT NULL,
    started_at              INTEGER,
    finished_at             INTEGER,
    cancel_requested        INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1)),
    canceled_at             INTEGER,
    claim_token             TEXT,
    claimed_at              INTEGER,
    builder_container_name  TEXT,
    builder_container_id    TEXT,
    sanitized_error         TEXT
);
CREATE INDEX idx_deployment_jobs_status_queued_at ON deployment_jobs(status, queued_at);
CREATE INDEX idx_deployment_jobs_application_status
    ON deployment_jobs(application_id, status, queued_at, id);
CREATE INDEX idx_deployment_jobs_run_id ON deployment_jobs(run_id);

CREATE TABLE build_configuration (
    id                    INTEGER PRIMARY KEY CHECK (id = 1),
    max_concurrent_builds INTEGER NOT NULL DEFAULT 3 CHECK (max_concurrent_builds > 0)
);

-- ── Git providers ───────────────────────────────────────────────────────

CREATE TABLE git_providers (
    id                            TEXT PRIMARY KEY,
    organization_id               TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name                          TEXT NOT NULL,
    provider_type                 TEXT NOT NULL
        CHECK (provider_type IN ('github', 'gitea', 'forgejo', 'gitlab', 'generic')),
    host_url                      TEXT NOT NULL DEFAULT '',
    username                      TEXT NOT NULL,
    password_encrypted            TEXT NOT NULL,
    auth_method                   TEXT NOT NULL DEFAULT 'token'
        CHECK (auth_method IN ('oauth2', 'token', 'gitlab_oauth')),
    kind                          TEXT NOT NULL DEFAULT 'credentials'
        CHECK (kind IN ('credentials', 'github_app')),
    created_at                    INTEGER NOT NULL,
    updated_at                    INTEGER NOT NULL,
    -- GitLab OAuth
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
    -- GitLab webhooks use a shared secret in `X-Gitlab-Token`.
    webhook_secret                TEXT
);
CREATE UNIQUE INDEX idx_git_providers_organization_name ON git_providers(organization_id, name);

CREATE TABLE github_apps (
    git_provider_id               TEXT PRIMARY KEY REFERENCES git_providers(id) ON DELETE CASCADE,
    app_id                        INTEGER,
    slug                          TEXT,
    owner_login                   TEXT NOT NULL DEFAULT '',
    owner_type                    TEXT NOT NULL DEFAULT 'user'
        CHECK (owner_type IN ('user', 'organization')),
    html_url                      TEXT NOT NULL DEFAULT '',
    api_base                      TEXT NOT NULL DEFAULT 'https://api.github.com',
    web_base                      TEXT NOT NULL DEFAULT 'https://github.com',
    private_key_encrypted         TEXT NOT NULL DEFAULT '',
    webhook_secret_encrypted      TEXT NOT NULL DEFAULT '',
    client_id                     TEXT NOT NULL DEFAULT '',
    client_secret_encrypted       TEXT NOT NULL DEFAULT '',
    installation_id               INTEGER,
    installation_token_encrypted  TEXT,
    installation_token_expires_at INTEGER,
    status                        TEXT NOT NULL DEFAULT 'pending_manifest'
        CHECK (status IN ('pending_manifest', 'pending_install', 'ready')),
    created_at                    INTEGER NOT NULL,
    updated_at                    INTEGER NOT NULL
);
CREATE INDEX idx_github_apps_installation ON github_apps(installation_id);

CREATE TABLE webhook_deliveries (
    id              TEXT PRIMARY KEY,
    git_provider_id TEXT NOT NULL REFERENCES git_providers(id) ON DELETE CASCADE,
    delivery_id     TEXT NOT NULL,
    event           TEXT NOT NULL,
    repo_full_name  TEXT,
    git_ref         TEXT,
    commit_sha      TEXT,
    result          TEXT NOT NULL,
    detail          TEXT,
    created_at      INTEGER NOT NULL
);
CREATE UNIQUE INDEX idx_webhook_deliveries_unique ON webhook_deliveries(git_provider_id, delivery_id);

-- ── Domains (server-wide Caddy/TLS settings) ───────────────────────────

CREATE TABLE server_domain_settings (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    domain     TEXT NOT NULL,
    email      TEXT NOT NULL,
    https      INTEGER NOT NULL DEFAULT 1 CHECK (https IN (0, 1)),
    updated_at INTEGER NOT NULL
);

-- ── Scheduled tasks ─────────────────────────────────────────────────────

CREATE TABLE scheduled_tasks (
    id                   TEXT PRIMARY KEY,
    organization_id      TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    server_id            TEXT REFERENCES servers(id) DEFAULT 'local',
    name                 TEXT NOT NULL,
    description          TEXT NOT NULL DEFAULT '',
    cron_expression      TEXT NOT NULL,
    runner               TEXT NOT NULL CHECK (runner IN ('ephemeral', 'remote_shell', 'litepod_api')),
    image                TEXT,
    shell                TEXT NOT NULL CHECK (shell IN ('bash', 'sh')),
    script               TEXT NOT NULL,
    timeout_seconds      INTEGER NOT NULL CHECK (timeout_seconds BETWEEN 1 AND 86400),
    network_access       INTEGER NOT NULL DEFAULT 0 CHECK (network_access IN (0, 1)),
    enabled              INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    next_run_at          INTEGER,
    active_execution_id  TEXT,
    created_by           TEXT NOT NULL REFERENCES users(id),
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    CHECK ((runner = 'ephemeral' AND image IS NOT NULL AND length(trim(image)) > 0) OR (runner = 'litepod_api' AND image IS NULL) OR (runner = 'remote_shell' AND image IS NULL))
);
CREATE INDEX idx_scheduled_tasks_due ON scheduled_tasks(enabled, next_run_at);
CREATE INDEX idx_scheduled_tasks_server ON scheduled_tasks(server_id);
CREATE INDEX idx_scheduled_tasks_organization ON scheduled_tasks(organization_id, created_at DESC);

CREATE TABLE scheduled_task_executions (
    id                TEXT PRIMARY KEY,
    task_id           TEXT NOT NULL REFERENCES scheduled_tasks(id) ON DELETE CASCADE,
    organization_id   TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    server_id         TEXT REFERENCES servers(id) DEFAULT 'local',
    trigger           TEXT NOT NULL CHECK (trigger IN ('scheduled', 'manual')),
    runner            TEXT NOT NULL CHECK (runner IN ('ephemeral', 'remote_shell', 'litepod_api')),
    image             TEXT,
    image_id          TEXT,
    shell             TEXT NOT NULL CHECK (shell IN ('bash', 'sh')),
    script            TEXT NOT NULL,
    timeout_seconds   INTEGER NOT NULL,
    network_access    INTEGER NOT NULL DEFAULT 0 CHECK (network_access IN (0, 1)),
    status            TEXT NOT NULL CHECK (status IN ('queued', 'running', 'succeeded', 'failed', 'canceled', 'replaced', 'interrupted')),
    container_name    TEXT,
    container_id      TEXT,
    process_group_id  INTEGER,
    queued_at         INTEGER NOT NULL,
    started_at        INTEGER,
    finished_at       INTEGER,
    exit_code         INTEGER,
    error             TEXT,
    log_bytes         INTEGER NOT NULL DEFAULT 0,
    logs_truncated    INTEGER NOT NULL DEFAULT 0 CHECK (logs_truncated IN (0, 1))
);
CREATE INDEX idx_scheduled_task_executions_task ON scheduled_task_executions(task_id, queued_at DESC);
CREATE INDEX idx_scheduled_task_executions_server ON scheduled_task_executions(server_id);
CREATE INDEX idx_scheduled_task_executions_organization
    ON scheduled_task_executions(organization_id, queued_at DESC);

CREATE TABLE scheduled_task_execution_logs (
    execution_id TEXT NOT NULL REFERENCES scheduled_task_executions(id) ON DELETE CASCADE,
    sequence     INTEGER NOT NULL,
    stream       TEXT NOT NULL CHECK (stream IN ('stdout', 'stderr', 'system')),
    message      TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    PRIMARY KEY (execution_id, sequence)
);

-- ── Notifications ───────────────────────────────────────────────────────

CREATE TABLE notification_providers (
    id               TEXT PRIMARY KEY,
    organization_id  TEXT NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name             TEXT NOT NULL,
    kind             TEXT NOT NULL
        CHECK (kind IN ('slack', 'discord', 'telegram', 'msteams')),
    enabled          INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    -- Emoji/colour decoration on the rendered message.
    decoration       INTEGER NOT NULL DEFAULT 1 CHECK (decoration IN (0, 1)),
    -- AES-256-GCM blob written by utils::security::encrypt_data. JSON inside:
    --   slack/discord/msteams -> {"webhook_url": "..."}
    --   telegram              -> {"bot_token": "...", "chat_id": "..."}
    config_encrypted TEXT NOT NULL,
    -- Redacted, safe-to-return display string, e.g.
    -- "hooks.slack.com/services/…/AB12". Never contains a usable credential.
    config_hint      TEXT NOT NULL DEFAULT '',
    created_by       TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL,
    UNIQUE (organization_id, name)
);
CREATE INDEX idx_notification_providers_organization ON notification_providers(organization_id);

CREATE TABLE notification_subscriptions (
    provider_id TEXT NOT NULL REFERENCES notification_providers(id) ON DELETE CASCADE,
    action      TEXT NOT NULL CHECK (action IN (
        'app_deploy',
        'app_build_error',
        'litepod_backup',
        'docker_cleanup',
        'litepod_restart'
    )),
    PRIMARY KEY (provider_id, action)
);

CREATE TABLE notification_deliveries (
    id              TEXT PRIMARY KEY,
    provider_id     TEXT NOT NULL REFERENCES notification_providers(id) ON DELETE CASCADE,
    action          TEXT NOT NULL,
    status          TEXT NOT NULL
        CHECK (status IN ('pending', 'sending', 'delivered', 'failed', 'dead')),
    attempts        INTEGER NOT NULL DEFAULT 0,
    next_attempt_at INTEGER NOT NULL,
    payload         TEXT NOT NULL,
    last_error      TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE INDEX idx_notification_deliveries_due ON notification_deliveries(status, next_attempt_at);
CREATE INDEX idx_notification_deliveries_provider
    ON notification_deliveries(provider_id, created_at DESC);

-- ── Backups ─────────────────────────────────────────────────────────────

CREATE TABLE s3_config (
    id                INTEGER PRIMARY KEY CHECK (id = 1),
    endpoint_url      TEXT NOT NULL,
    bucket            TEXT NOT NULL,
    region            TEXT NOT NULL DEFAULT 'us-east-1',
    access_key_id     TEXT NOT NULL,
    -- Secret access key, encrypted at rest (same shape as keystore.rs).
    wrapped_dek       BLOB NOT NULL,
    dek_nonce         BLOB NOT NULL,
    ciphertext        BLOB NOT NULL,
    nonce             BLOB NOT NULL,
    schedule_enabled  INTEGER NOT NULL DEFAULT 0 CHECK (schedule_enabled IN (0,1)),
    schedule_interval TEXT CHECK (schedule_interval IN ('daily','weekly')),
    schedule_time     TEXT,
    updated_at        INTEGER NOT NULL
);

CREATE TABLE backup_history (
    id            TEXT PRIMARY KEY,
    started_at    INTEGER NOT NULL,
    finished_at   INTEGER,
    status        TEXT NOT NULL CHECK (status IN ('running','success','failed')),
    size_bytes    INTEGER,
    s3_key        TEXT,
    error_message TEXT,
    triggered_by  TEXT NOT NULL CHECK (triggered_by IN ('manual','scheduled')),
    created_by    TEXT REFERENCES users(id) ON DELETE SET NULL
);
CREATE INDEX idx_backup_history_started_at ON backup_history(started_at DESC);

CREATE TABLE restore_history (
    id                TEXT PRIMARY KEY,
    backup_history_id TEXT NOT NULL REFERENCES backup_history(id) ON DELETE RESTRICT,
    started_at        INTEGER NOT NULL,
    finished_at       INTEGER,
    status            TEXT NOT NULL CHECK (status IN ('running','success','failed')),
    error_message     TEXT,
    created_by        TEXT REFERENCES users(id) ON DELETE SET NULL
);
CREATE INDEX idx_restore_history_started_at ON restore_history(started_at DESC);

-- ── Rate limiting ───────────────────────────────────────────────────────

CREATE TABLE rate_limit_settings (
    id                INTEGER PRIMARY KEY CHECK (id = 1),
    enabled           INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
    global_per_second INTEGER NOT NULL DEFAULT 20,
    global_burst      INTEGER NOT NULL DEFAULT 40,
    auth_per_second   INTEGER NOT NULL DEFAULT 5,
    auth_burst        INTEGER NOT NULL DEFAULT 10,
    auth_max_attempts INTEGER NOT NULL DEFAULT 5,
    auth_ban_minutes  INTEGER NOT NULL DEFAULT 15,
    updated_at        TEXT NOT NULL DEFAULT (datetime('now'))
);

-- ── Billing (Paddle) ────────────────────────────────────────────────────
-- Litepod Cloud is a single plan: a flat base fee plus a per-remote-server
-- fee. Paddle remains the payment authority; these tables provide
-- idempotency, authorization decisions and an auditable UI snapshot. Paddle
-- has no native usage-metering, so `billing_quantity_sync_attempts` records
-- every app-initiated attempt to keep a subscription's per-server item
-- quantity in sync with the live remote-server count.

CREATE TABLE billing_accounts (
    user_id                 TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    paddle_customer_id      TEXT UNIQUE,
    paddle_subscription_id  TEXT UNIQUE,
    paddle_product_id       TEXT,
    -- Mirrors base_price_id; kept for backward compatibility with any code
    -- still reading it, dropped in a future cleanup migration.
    paddle_price_id         TEXT,
    base_price_id           TEXT,
    per_server_price_id     TEXT,
    synced_server_quantity  INTEGER NOT NULL DEFAULT 0,
    quantity_synced_at      INTEGER,
    quantity_sync_error     TEXT,
    status                  TEXT NOT NULL DEFAULT 'inactive',
    period_starts_at        INTEGER,
    period_ends_at          INTEGER,
    grace_until             INTEGER,
    -- Superseded by the metered per-server model; kept at '{}' and dropped in
    -- a future cleanup migration once nothing reads it.
    entitlement_json        TEXT NOT NULL DEFAULT '{}',
    updated_at               INTEGER NOT NULL DEFAULT (unixepoch() * 1000)
);

CREATE TABLE billing_checkout_bindings (
    id                    TEXT PRIMARY KEY,
    user_id               TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    paddle_transaction_id TEXT UNIQUE,
    paddle_price_id       TEXT NOT NULL,
    created_at            INTEGER NOT NULL DEFAULT (unixepoch() * 1000),
    expires_at            INTEGER NOT NULL
);
CREATE INDEX idx_billing_checkout_bindings_user
    ON billing_checkout_bindings(user_id, created_at DESC);

CREATE TABLE billing_events (
    paddle_event_id TEXT PRIMARY KEY,
    event_type      TEXT NOT NULL,
    occurred_at     INTEGER NOT NULL,
    outcome         TEXT NOT NULL,
    payload_json    TEXT NOT NULL,
    attempt_count   INTEGER NOT NULL DEFAULT 0,
    last_attempt_at INTEGER,
    last_error      TEXT,
    received_at     INTEGER NOT NULL DEFAULT (unixepoch() * 1000)
);
CREATE INDEX idx_billing_events_pending ON billing_events(outcome, received_at);

CREATE TABLE billing_invoices (
    paddle_transaction_id  TEXT PRIMARY KEY,
    user_id                TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    paddle_subscription_id TEXT,
    invoice_number         TEXT,
    status                 TEXT NOT NULL,
    amount                 TEXT NOT NULL,
    currency_code          TEXT NOT NULL,
    issued_at              INTEGER,
    document_url           TEXT,
    updated_at              INTEGER NOT NULL DEFAULT (unixepoch() * 1000)
);
CREATE INDEX idx_billing_invoices_user ON billing_invoices(user_id, issued_at DESC);

CREATE TABLE billing_webhook_deliveries (
    id                   TEXT PRIMARY KEY,
    paddle_event_id      TEXT,
    event_type           TEXT,
    request_headers_json TEXT NOT NULL,
    request_body         TEXT NOT NULL,
    response_status      INTEGER,
    response_body        TEXT,
    error_message        TEXT,
    received_at          INTEGER NOT NULL DEFAULT (unixepoch() * 1000),
    completed_at         INTEGER
);
CREATE INDEX idx_billing_webhook_deliveries_received_at
    ON billing_webhook_deliveries(received_at DESC);

CREATE TABLE billing_reconciliation_runs (
    id                    TEXT PRIMARY KEY,
    triggered_by_user_id  TEXT REFERENCES users(id),
    started_at            INTEGER NOT NULL DEFAULT (unixepoch() * 1000),
    completed_at          INTEGER,
    processed_count       INTEGER NOT NULL DEFAULT 0,
    pending_count         INTEGER NOT NULL DEFAULT 0,
    failed_count          INTEGER NOT NULL DEFAULT 0,
    candidate_count       INTEGER NOT NULL DEFAULT 0,
    unassigned_count      INTEGER NOT NULL DEFAULT 0,
    error_message         TEXT
);

CREATE TABLE billing_external_records (
    id                      TEXT PRIMARY KEY,
    paddle_subscription_id  TEXT UNIQUE,
    paddle_customer_id      TEXT UNIQUE NOT NULL,
    customer_email          TEXT,
    normalized_email        TEXT,
    paddle_product_id       TEXT,
    paddle_price_id         TEXT,
    -- JSON array of {price_id, role, quantity, product_id} captured from the
    -- Paddle subscription's items — the base+per_server pair, not just one.
    items_json               TEXT,
    subscription_status      TEXT NOT NULL,
    state                    TEXT NOT NULL CHECK (state IN ('candidate_match','unassigned_external','assigned','ignored')),
    candidate_user_id        TEXT REFERENCES users(id),
    assigned_user_id         TEXT REFERENCES users(id),
    assigned_by_user_id      TEXT REFERENCES users(id),
    discovered_at            INTEGER NOT NULL DEFAULT (unixepoch() * 1000),
    assigned_at              INTEGER,
    updated_at               INTEGER NOT NULL DEFAULT (unixepoch() * 1000)
);
CREATE INDEX idx_billing_external_records_state ON billing_external_records(state, discovered_at DESC);

CREATE TABLE billing_quantity_sync_attempts (
    id                      TEXT PRIMARY KEY,
    user_id                 TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    paddle_subscription_id  TEXT NOT NULL,
    previous_quantity       INTEGER NOT NULL,
    target_quantity         INTEGER NOT NULL,
    proration_mode          TEXT NOT NULL,
    status                  TEXT NOT NULL CHECK (status IN ('success', 'failed')),
    error_message           TEXT,
    attempted_at            INTEGER NOT NULL DEFAULT (unixepoch() * 1000)
);
CREATE INDEX idx_billing_quantity_sync_attempts_user
    ON billing_quantity_sync_attempts(user_id, attempted_at DESC);
