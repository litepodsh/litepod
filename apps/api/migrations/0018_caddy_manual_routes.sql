-- Standalone Caddy routes not tied to a litepod application: a hostname that
-- either reverse-proxies to an upstream or redirects to a URL. Managed from the
-- Caddy console page. Central proxy only for now (server_id = 'local').

CREATE TABLE caddy_manual_routes (
    id              TEXT PRIMARY KEY,
    server_id       TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    host            TEXT NOT NULL,
    kind            TEXT NOT NULL CHECK (kind IN ('reverse_proxy', 'redirect')),
    target          TEXT NOT NULL,
    https           INTEGER NOT NULL DEFAULT 1 CHECK (https IN (0, 1)),
    redirect_status INTEGER NOT NULL DEFAULT 308 CHECK (redirect_status IN (301, 302, 307, 308)),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_caddy_manual_routes_host ON caddy_manual_routes(host);
CREATE INDEX idx_caddy_manual_routes_server ON caddy_manual_routes(server_id, created_at);
