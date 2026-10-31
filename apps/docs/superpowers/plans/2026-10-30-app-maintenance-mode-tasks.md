# Per-app maintenance mode — task list

Goal: an app admin turns on maintenance for an app. Visitors of its domains get a
503 page instead of the app. The containers keep running. Turning it off puts the
normal route back at once.

## Decisions (agreed in chat unless marked "my call")

- Visitors see a **503 page**, either the **default template** or the user's own **HTML**.
- **Containers keep running** (nothing stops, nothing redeploys). A "stop containers
  too" switch is a possible later option, not in this version.
- Covers **all domains** of the app.
- **Phase 1: a single HTML document** (pasted or uploaded as `.html`). Phase 2: a ZIP
  with assets, reusing `deployment/source_archive.rs` validation. Not in this task list.
- Works for any app, however it was built (Railpack, ZIP source, git, image, Compose).
- My call: the page is stored in the database (cap 512 KiB), not on disk, so it travels
  with the config to wherever Caddy runs.
- My call: response headers `Cache-Control: no-store`, `Retry-After: 300`,
  `X-Robots-Tag: noindex`, so search engines and CDNs do not keep a maintenance page.
- My call: basic auth does not run while maintenance is on (everyone sees the page).
- My call: **public-edge servers are not covered yet.** Their routes are built by the
  embedded edge helper script (`servers/setup.rs`, versioned in `servers/helper.rs`),
  which needs its own change and a version bump. Until then, turning the mode on for
  such an app is refused with a clear message instead of silently doing nothing.
  Central Caddy and private-relay apps are covered.

## Tasks

### Backend
- [x] Migration `0077_application_maintenance.sql`
- [x] `deployment/maintenance.rs`: default page, get/update, size cap, payload loader
- [x] Caddy `static_response` handle; escape `{`/`}` so Caddy does not expand placeholders
- [x] `apply_app_domain_to_config`: maintenance replaces the proxy (and auth) handle
- [x] Load the payload in `caddy_route`, `caddy_application_routes`, route recovery
- [x] Reconcile every domain of the app when the mode changes (`reconcile_domain_route`,
      the strict twin of `reconcile_caddy_basicauth`; a failed route restores the old state)
- [x] Refuse enabling on public-edge apps with a clear message
- [x] Handlers + routes (`GET`/`PUT .../maintenance`) with `RequireApp`
- [x] Audit-log entry on change (`maintenance.enabled` / `.disabled` / `.page_updated`)
- [x] Backend tests: route shape, escaping, defaults, page kept while off, other app unaffected,
      refused on public-edge, oversized page
  - `cargo test`: 1125 pass; same single failure as the 2FA task (`tenancy_coverage`, telemetry).
  - Not tested: surviving a rollout end to end. `caddy_application_routes` reads the payload by
    domain id (same path as basic auth), but no test drives it against a Caddy admin API.

### UI
- [x] `project-client.ts`: maintenance client + types (+ 2 tests in `project-client.test.ts`)
- [x] App settings card (Advanced tab): switch with confirmation, default vs custom HTML,
      load an `.html` file, sandboxed preview (`application-maintenance-card.svelte`)
- [x] "In maintenance" badge in the application detail header, next to Running/Stopped
      (read on load; the card updates it when switched). Not on the application lists.
- [x] i18n bug fixed: `projects.maintenance.title` was missing in en and es.

### MCP / assistant
- [x] Action `set_maintenance` (`enabled`, optional `page_html`): allowlist, proposal, apply
      through `deployment::maintenance::update`, tool schema and help text, audit.
      Over MCP it is auto-confirmed as the token's user (not `MCP_DIRECT`, which a test pins to
      start/stop). In the in-app chat it stays a proposal a person confirms.
- [x] Tests: stored until confirmed, off keeps the page, oversized page refused, allowlist.
- [x] `#![recursion_limit = "256"]` in `main.rs`: the `exec` schema `json!` hit the limit.

### Stage 1: ZIP site served from files (done)
- [x] Migration `0078` (`site_id`); `source_archive::unpack_bytes` with its own limits
      (10 MB, 500 files, 20 MB unpacked), refactor of `inspect` covered by the old tests.
- [x] `maintenance::upload_site` / `remove_site`: unpack to a scratch dir, require `index.html`,
      swap in only when valid; a bad upload leaves the serving site alone.
- [x] Caddy route for a site: `rewrite` onto `/__litepod/maintenance/{app}` + `reverse_proxy` to this
      API, `Host` untouched. Falls back to the inline page when the files are gone.
- [x] Public route `handler/maintenance_site.rs`, mounted outside the rate limiter. Serves only while
      in maintenance and only to the application's own `Host`; document 503, assets 200; path
      traversal and symlinks refused; `no-store`, `noindex`, `nosniff`.
- [x] Endpoints `POST`/`DELETE .../maintenance/site`, audit `maintenance.site_uploaded|removed`.
- [x] UI: source selector (default / HTML / ZIP site), upload, file summary. i18n en + es.
- [x] Tests: 19 maintenance tests; `cargo test` 1136 pass, same single unrelated failure.
- [ ] Not tested: a real Caddy rewriting onto the route, and a browser loading a site through it.

### Stage 2: Railpack-built page, container stopped until maintenance is on
Designed and **approved**: `docs/superpowers/specs/2026-10-30-maintenance-railpack-design.md`.
- [x] Verified on this machine: Railpack plan for a lone `index.html` (Caddy, port 80, `/health`),
      forced 503 via `handle_response` + `copy_response`, header placement, `errors` fallback.
- [x] Hidden companion plumbing: migrations 0079 (`maintenance_for`) and 0080 (`companion_status`,
      `companion_error`); `deployment/maintenance_companion.rs` creates it on ZIP upload (Railpack,
      port 80, same server, 1 CPU / 1 GiB build limits, no domains), queues the build, **parks** the
      container with `podman stop` (never removed; `desired_state = stopped`, boot policy `restore`),
      records `building / ready / failed`, and tears it down on site removal or parent delete.
- [x] Guard test `maintenance_companion_guard.rs`: every query over `applications` must filter
      `maintenance_for IS NULL`, go by one id, or be allowed with a written reason. Verified it
      fails when a filter is removed.
- [x] UI: drag-and-drop ZIP zone (same component as the General tab), build status line.
- [x] Turn-on / turn-off (`maintenance_companion::activate/deactivate`, migration 0081 `companion_live`):
      on = `podman start`, wait for `GET /health`, only then set live and move the route; any failure
      stops it again and the static copy serves. Off = route back first, then `podman stop`.
      A rebuild takes the page out of service (static copy) and puts the new build back when parked.
- [x] Route shape: `headers` handler, then `reverse_proxy` with `handle_response` forcing 503 on
      `text/html*` (assets stay 200), plus a per-host `errors` route with the inline page. Removed on
      domain delete and orphan prune. Tested.
- [x] Verified with real Podman: `stop` leaves the container `exited`, `start` answers in ~220 ms and
      the published port is unchanged.
- [ ] Recreate a parked container that is missing (today: falls back to the static copy; upload
      the ZIP again to rebuild).
- [ ] Companion on a remote server (today: static copy only).
- [ ] Verify a real Railpack build (needs BuildKit) and a real end-to-end switch through Caddy.
