-- Per-application HTTP/TCP health check, separate from the Podman-level
-- `restart_policy: unless-stopped` that already recovers a crashed process:
-- this catches a process that is still running but no longer answering.
CREATE TABLE application_health_checks (
    application_id        TEXT PRIMARY KEY REFERENCES applications(id) ON DELETE CASCADE,
    enabled                INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    protocol               TEXT NOT NULL DEFAULT 'http' CHECK (protocol IN ('http', 'tcp')),
    path                   TEXT NOT NULL DEFAULT '/',
    port                   INTEGER,
    interval_seconds       INTEGER NOT NULL DEFAULT 30 CHECK (interval_seconds BETWEEN 5 AND 3600),
    timeout_seconds        INTEGER NOT NULL DEFAULT 5 CHECK (timeout_seconds BETWEEN 1 AND 60),
    unhealthy_threshold    INTEGER NOT NULL DEFAULT 3 CHECK (unhealthy_threshold BETWEEN 1 AND 20),
    restart_on_unhealthy   INTEGER NOT NULL DEFAULT 0 CHECK (restart_on_unhealthy IN (0, 1)),
    consecutive_failures   INTEGER NOT NULL DEFAULT 0,
    last_status            TEXT NOT NULL DEFAULT 'unknown' CHECK (last_status IN ('unknown', 'healthy', 'unhealthy')),
    last_checked_at        INTEGER,
    last_error             TEXT,
    last_restarted_at      INTEGER,
    created_at             INTEGER NOT NULL,
    updated_at             INTEGER NOT NULL
);

CREATE INDEX idx_application_health_checks_due ON application_health_checks(enabled, last_checked_at);
