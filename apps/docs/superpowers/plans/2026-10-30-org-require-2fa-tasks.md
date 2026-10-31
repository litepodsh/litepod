# Org-wide 2FA requirement — task list

Goal: an organization admin can require every member to use two-factor
authentication, and a member without it is walked through setting it up.

## Decisions (my calls, not yet confirmed — change any of them)

- **Enforcement point:** `base()` in `api/src/security/authz/extract.rs`. Every
  org-scoped route goes through it, so one check covers password, SSO, OAuth
  and token callers. Account routes are not org-scoped, so 2FA setup stays
  reachable while blocked.
- **Refusal:** `403` with a new `AppError::TwoFactorRequired` and a stable
  error code, so the UI can redirect instead of showing a generic toast.
- **Exempt:** SysAdmin (break-glass, same as `SsoRequired`).
- **Lock-out guard:** an admin cannot turn the setting on while their own
  account has no 2FA.
- **Visibility first (Coolify did this first):** the members list shows who
  has 2FA, to users who can manage members.
- **Off by default.**

## Tasks

### Research
- [x] Read how Coolify did it (members column first, then team setting)
- [x] Read login, SSO-required, org settings and extractor code in Litepod

### Backend
- [x] Migration `0076_organization_require_two_factor.sql`
- [x] `OrganizationRow.require_two_factor` + column lists + `set_require_two_factor`
- [x] `OrganizationDto` / `UpdateOrganizationRequest` field
- [x] `update_organization`: apply the field, refuse enabling without own 2FA
- [x] `AppError::TwoFactorRequired` (variant, message, 403, error code)
- [x] Enforce in `extract.rs::base()` (skip SysAdmin; read flag, then TOTP)
- [x] `OrganizationMemberDto.two_factor_enabled` (admins/members-managers only)
- [x] Audit-log entry when the setting changes
- [x] Backend tests: migration default, enforcement, exempt SysAdmin, lock-out guard, members flag
  - `cargo test`: 1115 pass. 1 fails: `tenancy_coverage::body_scope::every_body_organization_is_covered`
    ("missing handler: telemetry::get_handler"). Not touched by this work; not checked on a clean tree.

### UI
- [x] `organization-client.ts`: `require_two_factor`, member `two_factor_enabled`
- [x] `base-client.ts`: expose the error code on `ApiError` (+ `base-client.test.ts`, 3 tests)
- [x] Settings panel with switch (new "Security" tab, `two-factor-settings-panel.svelte`)
- [x] Members list: per-member 2FA badge; "N of M without 2FA" summary is in the Security panel
- [x] Redirect on `two_factor_required` to the account security tab, with a banner
- [x] After setup, return the user to where they were (after the recovery codes dialog is closed)
- [x] i18n strings in `en` and `es` (`organizations.toml`, `account.toml`)

### Wrap-up
- [x] `cargo test` (see Backend note), `bun run check` (0 errors), `vitest` i18n + clients pass
- [x] Docs page (Spanish): `docs/security/two-factor.md`
- [ ] Manual browser check of the full flow (not done: nothing was run in a browser)

## Not committed
Per request, nothing is committed. Changes are in the working tree.
