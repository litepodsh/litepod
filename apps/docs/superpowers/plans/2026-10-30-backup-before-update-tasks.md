# Backup before update — tasks

- [x] Migration 0082 `backup_s3_targets.backup_before_update` (default 0); config load/save/DTO.
- [x] `backup::run::wait_for` (success only; failed/missing/overlong = error).
- [x] `POST /system/update/apply`: starts the backup synchronously (409 if it cannot start), the
      task waits for it and cancels the update on failure (note `update cancelled: ...`);
      response carries `backup_started`.
- [x] UI: switch in the backups panel; update store waits without the 20 s grace timer, shows
      the cancel reason, reloads on version change; i18n en/es.
- [x] Tests: `wait_for`, flag persistence, store wait and cancel.
- [x] Docs `docs/containers/backup-before-update.md`.
- [ ] Real run: enable, press Update on a dev stack with S3, confirm order backup → pull → recreate.
- [ ] Use a dedicated `triggered_by = 'update'` (needs table rebuild; today recorded as `manual`).
