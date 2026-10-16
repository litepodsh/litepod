-- Isolated deployment candidates use OS-assigned loopback ports. Preserve
-- placement and uniqueness constraints while admitting these dynamic ports.
DROP TRIGGER application_runtime_endpoints_require_remote_placement_insert;
DROP TRIGGER application_runtime_endpoints_require_remote_placement_update;

ALTER TABLE application_runtime_endpoints RENAME TO application_runtime_endpoints_fixed_ports;

CREATE TABLE application_runtime_endpoints (
    application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
    server_id      TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    host_port      INTEGER NOT NULL CHECK (host_port BETWEEN 1024 AND 65535),
    container_port INTEGER NOT NULL CHECK (container_port BETWEEN 1 AND 65535),
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    PRIMARY KEY (application_id, container_port),
    UNIQUE (server_id, host_port)
);

INSERT INTO application_runtime_endpoints
    (application_id, server_id, host_port, container_port, created_at, updated_at)
SELECT application_id, server_id, host_port, container_port, created_at, updated_at
FROM application_runtime_endpoints_fixed_ports;

DROP TABLE application_runtime_endpoints_fixed_ports;

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

