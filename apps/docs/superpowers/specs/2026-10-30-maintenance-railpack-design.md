# Maintenance page built by Railpack — design

Status: **approved** (2026-10-30). Verification done (see below); implementation not started.
Stage 1 (below) is built and tested.

## Intent

A user uploads a ZIP as the maintenance page of an application. Litepod builds it with
Railpack, keeps the resulting container **stopped**, and only when maintenance is turned on
starts it and **moves the application's domains to point at that container**. Turning
maintenance off points them back at the application and stops the container again.

Why: a plain HTML file or static ZIP is not enough for someone whose maintenance page is a
small project (Vite, Astro, a Node page). Railpack already detects those.

## What exists today (stage 1, built)

| Source | How it is served |
|---|---|
| Default / inline HTML | Caddy `static_response`, 503 |
| ZIP site (static files) | Caddy rewrites onto `/__litepod/maintenance/{app}/...`, this API serves the files; the HTML document is a 503, assets are 200; only for the application's own `Host` |

Both survive deploys, route recovery and basic-auth changes because routes are rebuilt from the
database (`maintenance::load_payload_for_domain`).

## The decision

The Railpack pipeline (`deployment/git/railpack.rs`, the queue, rollout, run history) is keyed by
an `applications` row: `/data/repos/{application_id}`, image `litepod-app-{application_id}`.
There is no way to build "just a container". So the page becomes a **companion application**:
a hidden `applications` row, one per parent, built and run by the existing machinery.

This touches 39 files / 79 queries that read `applications`. That is the real cost, and the
main risk: a companion that leaks into a list, an analytics view or a limit is a ghost app.

## Safety properties (what "safe" means here)

1. **Never public by itself.** The companion has no domains, no published host ports, and is
   reachable only through the parent's domain while maintenance is on.
2. **Visitors never get a 502 or the application by accident.** Turning on:
   start the companion → wait until it is healthy → only then repoint the route. If any step
   fails, the route goes to the stage-1 page (ZIP files if present, else inline, else default),
   maintenance stays on, and the error is reported. Turning off: repoint to the application
   first, stop the companion after. A failed stop never affects visitors.
3. **A bad build touches nothing.** A failed build keeps the previous good build, never changes
   the parent or its route, and the failure is visible on the card.
4. **The ZIP is the same ZIP.** One upload feeds both stage 1 (served from files immediately,
   usable before the build ends) and the Railpack build. Same limits: 10 MB, 500 files, 20 MB
   unpacked, safe paths, `index.html` required.
5. **Small and capped.** Companion runtime and build limits are fixed low (CPU, memory), set
   through the existing `runtime_*` / `build_*` columns; it cannot be raised by the user.
6. **Same privilege as deploying an app, no more.** Railpack runs the project's own build
   commands, exactly as for any application the user can already deploy. Requires the same
   permission as editing the application (`app.update`). Nothing runs at upload time on the
   Litepod host outside the existing builder container.
7. **Gone with its parent.** `ON DELETE CASCADE` on the link, plus teardown of its container and
   image. Exactly one per parent (unique index). Never in a list, a quota, analytics, the
   assistant's `get applications`, or MCP.
8. **Truth comes from the runtime, not the row.** After a Litepod restart, route recovery
   decides "container or fallback" from the companion's real state (running, healthy), not from
   what the database last said.
9. **No toggle races.** One lock per application for turn-on / turn-off / rebuild.

## Architecture

- Migration: `applications.maintenance_for TEXT REFERENCES applications(id) ON DELETE CASCADE`
  + unique index; `application_maintenance.companion_status` (`none | building | ready | failed`)
  and `companion_error`.
- Upload (`POST .../maintenance/site`): validate, store the ZIP, create the companion if missing
  (`source_upload = 1`, `deploy_type = git`, `build_method = railpack`, tight limits), enqueue a
  build through the normal queue. When the build finishes, stop the container (state `ready`).
- Turn on: stage-1 route first (instant, safe), then in the background start the companion, wait
  healthy, repoint the route to `container:port` with the **503 forced** on the response. A
  failure leaves the stage-1 route in place.
- Route shape: `reverse_proxy` to the companion with `handle_response` → `copy_response` and
  `status_code: 503` for HTML documents only (assets stay 200), plus the no-store / noindex headers.
- Turn off: route back to the application, then stop the companion.
- Remote servers: the companion lives on the parent's server and is dialed the way the parent
  is. **Public-edge servers stay unsupported** (same reason as stage 1).

## Verified on this machine (2026-10-30, Podman + the repo's own builder Dockerfile + Caddy 2.11.4)

All in throwaway `lp-verify-*` containers on a private network; the running `litepod-caddy` was
never touched and everything created was removed afterwards.

1. **Railpack and a plain `index.html`: confirmed.** `railpack prepare` (v0.31.1, the version
   pinned in `Dockerfile.railpack-builder`) turns a directory with only an `index.html` into a
   static site served by Caddy: `caddy run --config Caddyfile`. It listens on **`:{$PORT:80}`
   (port 80 by default)** and answers **`/health` with 200**, which is what the health wait uses.
   It adds its own strict CSP header and gzip/zstd. Only the *plan* was generated; the real
   build needs BuildKit, which is not running here, so a full build is still unseen.
2. **Forcing the 503: confirmed, with a different shape than first sketched.**
   `reverse_proxy` with `handle_response` matching `Content-Type: text/html*`, `status_code:
   "503"` and a `copy_response` route gives an HTML document of 503 with its body intact, and
   the CSS of 200. Response headers (`Cache-Control: no-store`, `Retry-After`, `X-Robots-Tag`)
   only apply when a `headers` handler sits **before** the `reverse_proxy` in the route; inside
   `handle_response` they were silently dropped.
3. **A live fallback exists, with no orchestration: confirmed.** A server-level `errors` route
   matching the same host answers an inline 503 page when the companion is stopped or dies
   *while maintenance is on*. This closes the gap "container died after the route moved", which
   the start-wait-switch ordering alone cannot. It adds one more Caddy object per domain
   (`errors.routes`) that deletion and orphan pruning must clean up.
4. **Not verified:** a real Railpack build and start of the companion (needs BuildKit), and its
   start-to-healthy time. Static Caddy starts in about a second, so the stage-1 page covers it,
   but that is a guess until a build runs.

## Changes to the design from what was found

- Companion `container_port` is **80** (Railpack's static default), not configurable by the user.
- Health wait uses `GET /health` on the companion.
- Route shape: `headers` handler, then `reverse_proxy` with the `handle_response` 503 override,
  plus the per-host `errors` fallback route.
- Hidden-companion filtering: about **34** reads of `applications` are not by a single id (the
  rest are by-id and cannot list one). They are the ones to audit: dashboard, organization app
  lists and summaries, server counts, metrics, analytics, notifications, the assistant's `get
  applications`, queue joins. A test must fail when one of them forgets the filter.

## Tasks

- [x] Verify Railpack static plan, forced 503, header placement and the `errors` fallback
- [ ] Verify a real build and start of the companion (needs BuildKit) and its start-to-healthy time
- [ ] Migration + hidden-companion plumbing; filter it out of the ~34 non-by-id `applications`
      reads, with a test that fails when a read forgets the filter
- [ ] Create / rebuild / tear down the companion; stop after build; tight limits
- [ ] Turn-on ordering with fallback; turn-off ordering; per-application lock
- [ ] Forced-503 route shape; route recovery from real runtime state
- [ ] Card: say the ZIP is built with Railpack, shows build status and the error, and that the
      container stays stopped until maintenance is on
- [ ] Tests for every safety property above; docs

## Approaches considered

1. **Companion application (this design).** Reuses the build, run and health machinery. Costs the
   hidden-row audit.
2. **Build only, no row.** Would mean duplicating the Railpack pipeline for a row-less build.
   Rejected: a second pipeline to keep in sync with the first.
3. **Stay on stage 1.** Static ZIPs only, no build. Already built; what this design extends.
