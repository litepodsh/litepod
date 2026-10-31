# Changelog

Notable changes per release. Release notes on GitHub are generated from commits; this file keeps
the user-facing summary and the migrations each release adds.

## Unreleased

### Added

- **Organization-wide two-factor requirement.** Owners can require 2FA for every member
  (Organization settings). Members without 2FA get `two_factor_required` (403) until they enable
  it; system admins are exempt. Migration `0076`. Docs: `apps/docs/security/two-factor.md`.
- **Per-application maintenance mode.** Serve a maintenance page with HTTP 503 and `Retry-After`
  on the application's domains while it is down. Page sources: default template, inline HTML, or a
  ZIP site. A ZIP is also compiled with Railpack into a hidden, stopped companion container that
  is started and health-checked before domains are switched, with a static fallback so visitors
  never get a 502. The state shows in the application header and can be set through MCP
  (`set_maintenance`). Migrations `0077`–`0081`. Docs: `apps/docs/security/maintenance-mode.md`.
- **Container file explorer.** Browse, edit, create, upload and download files in a running
  container from the Containers tab. Requires `app.exec`; every change is audited
  (`container.file_*`). Docs: `apps/docs/containers/file-explorer.md`.
- **Back up before updating Litepod.** Optional switch (Settings → Backups): applying an update
  first saves an instance backup and cancels the update if the backup fails. Migration `0082`.
  Docs: `apps/docs/containers/backup-before-update.md`.

### Changed

- The application topology map tab and its graph layout code were removed.
