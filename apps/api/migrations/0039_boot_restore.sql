-- Boot restore (docs/superpowers/specs/2026-09-26-boot-restore-design.md).
--
-- Rootless Podman has no daemon and `podman-restart.service` only starts
-- `restart=always` containers, so after a reboot every app stays `Exited` and the
-- monitor rewrites `status` to `stopped`. `desired_state` keeps what the user
-- wants apart from what Podman reports; only a user stop/remove writes `stopped`.
PRAGMA foreign_keys = OFF;

BEGIN;

ALTER TABLE applications ADD COLUMN boot_policy TEXT NOT NULL DEFAULT 'restore'
    CHECK (boot_policy IN ('restore', 'always', 'notify'));
ALTER TABLE application_deployments ADD COLUMN desired_state TEXT NOT NULL DEFAULT 'stopped'
    CHECK (desired_state IN ('running', 'stopped'));
-- NULL until the first probe; a NULL only records the id and never triggers a
-- restore, so upgrading litepod cannot start every app at once.
ALTER TABLE servers ADD COLUMN last_boot_id TEXT;

-- Deployed apps are wanted running unless the last user action on them was a stop.
UPDATE application_deployments
SET desired_state = 'running'
WHERE status <> 'not_deployed'
  AND COALESCE((
        SELECT l.action FROM audit_logs l
        WHERE l.target_type = 'application'
          AND l.target_id = application_deployments.application_id
          AND l.action IN ('deployment.started', 'deployment.stopped')
        ORDER BY l.id DESC
        LIMIT 1
      ), 'deployment.started') = 'deployment.started';

-- Eight deploy/start paths write `status = 'running'`; one trigger records the
-- intent for all of them. The monitor seeing a container up counts too.
CREATE TRIGGER trg_application_deployments_desired_running
AFTER UPDATE OF status ON application_deployments
WHEN NEW.status = 'running' AND NEW.desired_state <> 'running'
BEGIN
    UPDATE application_deployments SET desired_state = 'running'
    WHERE application_id = NEW.application_id;
END;

-- SQLite cannot alter a CHECK in place.
CREATE TABLE notification_subscriptions_new (
    provider_id TEXT NOT NULL REFERENCES notification_providers(id) ON DELETE CASCADE,
    action      TEXT NOT NULL CHECK (action IN (
        'app_deploy',
        'app_build_error',
        'litepod_backup',
        'docker_cleanup',
        'litepod_restart',
        'server_reboot'
    )),
    PRIMARY KEY (provider_id, action)
);
INSERT INTO notification_subscriptions_new SELECT provider_id, action FROM notification_subscriptions;
DROP TABLE notification_subscriptions;
ALTER TABLE notification_subscriptions_new RENAME TO notification_subscriptions;

COMMIT;

PRAGMA foreign_keys = ON;
