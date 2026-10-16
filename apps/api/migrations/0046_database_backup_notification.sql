-- SQLite cannot alter a CHECK in place: rebuild notification_subscriptions to
-- accept the new `database_backup` action (per-database backup completion).
PRAGMA foreign_keys = OFF;

BEGIN;

CREATE TABLE notification_subscriptions_new (
    provider_id TEXT NOT NULL REFERENCES notification_providers(id) ON DELETE CASCADE,
    action      TEXT NOT NULL CHECK (action IN (
        'app_deploy',
        'app_build_error',
        'litepod_backup',
        'docker_cleanup',
        'litepod_restart',
        'server_reboot',
        'database_backup'
    )),
    PRIMARY KEY (provider_id, action)
);
INSERT INTO notification_subscriptions_new SELECT provider_id, action FROM notification_subscriptions;
DROP TABLE notification_subscriptions;
ALTER TABLE notification_subscriptions_new RENAME TO notification_subscriptions;

COMMIT;

PRAGMA foreign_keys = ON;