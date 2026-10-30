<div align="center">

<a href="https://litepod.sh">
  <img src="assets/litepod.svg" alt="Litepod" width="88" height="88">
</a>

# Litepod

**Host your apps. Keep control.**

Litepod is an open-source, self-hostable PaaS built on Podman — a lightweight alternative to Heroku, Vercel and Netlify.
Deploy applications, databases and services to servers you own, with a clean panel instead of a black box.

[Website](https://litepod.sh) · [Docs](https://docs.litepod.sh) · [Live demo](https://demo.litepod.sh) · [Templates](https://templates.litepod.sh) · [Litepod Cloud](https://app.litepod.sh/register?utm_source=github&utm_medium=readme&utm_campaign=cloud) · [Roadmap](ROADMAP.md)

**English** · [Español](README.es.md)

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/screenshots/home-dark.webp">
  <img src="assets/screenshots/home-light.webp" alt="Litepod home: projects, containers and recent activity at a glance">
</picture>

</div>

## Why Litepod

Most self-hosted PaaS tools are either a thin wrapper around Docker or a heavy platform that needs its own server just to run. Litepod sits in between:

- **Podman, not Docker.** Daemonless and rootless by default. No privileged socket exposed to the internet.
- **Tiny footprint.** A fresh install runs in about **63 MB of RAM** across three containers (measured with `podman stats`).
- **Rust + SvelteKit.** An axum API on SQLite and a fast Svelte 5 panel. No Postgres or Redis required for the control plane.
- **Your machine, your data.** Everything runs on infrastructure you control. No lock-in: export, move or self-host at any time.

## Features

- **Git deployments** — push to deploy from GitHub, with build logs streamed live. Builds with Railpack (zero-config), a `Dockerfile`, or Compose.
- **Compose-native** — deploy multi-container stacks from a `compose.yml` as a single application.
- **One-click templates** — start from a known stack, set the variables, and ship. Browse them at [templates.litepod.sh](https://templates.litepod.sh).
- **Databases** — provision databases alongside your apps, with persistent volumes.
- **Automatic HTTPS** — custom domains with TLS certificates issued and renewed by Caddy.
- **Multi-server fleet** — connect remote Debian, Ubuntu, Fedora or Arch servers over key-only SSH. Use a public edge, a private relay, or keep services private.
- **Observability** — request analytics, errors and latency through the proxy, plus CPU and memory per application. Analytics and metrics are stored in an embedded [DuckDB](https://duckdb.org) database _(beta)_.
- **Web terminal** — open a shell inside any container from the browser.
- **Volumes** — inspect and edit persistent storage and live mounts.
- **Backups** — scheduled backups to any S3-compatible bucket.
- **Teams** — organizations, members and roles.

## MCP

Connect your own agent (Claude, Cursor, …) to `/api/v1/organizations/{organization_id}/mcp`. It exposes two tools:

- **`get`** reads. The organizations the user belongs to; every project and application in the organization with its status (`running`, `stopped`, `failed`, …) in one call, or one project; every project's environments with the production one marked; every domain in the organization, or one application's; plus environment variables with their values, deployments and their logs, live containers and their logs, source, compose file, servers and Git providers; the internal database templates and the template catalog at [templates.litepod.sh](https://templates.litepod.sh) — searched by name, tag or category, and for one template its ports and the variables the user has to fill in. When the catalog has more than one template for what the user asked for, the agent gets the candidates and asks which one they want.
  - **Environments** — reads default to the production environment. Pass `environment` (e.g. `qa`) to read another one, or `environment: "all"` to list every environment. It also picks between same-named applications in different environments.
  - **`restart`** — restarts one container, or the whole application; if nothing is running it redeploys instead.
  - **`rollback`** — redeploys an earlier successful run (from `get deployments`), or the one before the current deploy when none is given.
  - **`repositories`** — lists and searches the repositories the organization's Git providers can see (private ones included), or the branches of one repository with `git_url`.
  - **`url`** — reads one public HTTPS page as text (a README, a `compose.yml`); a github.com repository or file address returns the README or the raw file. Responses are size-capped and marked as untrusted data, and internal addresses are refused.
  - **`web_search`** — searches the web through the provider the organization set up in the assistant settings: a SearXNG instance it runs, or a Brave Search API key. Off until an owner adds one; the key is encrypted and never shown again (it can only be turned off or deleted). Results are marked as untrusted data. Over MCP it only appears once an owner has set it up and turned it on: your agent's own web search is the default.
  - **`wait_deployment`** — blocks up to 55 seconds until an application has no deploy queued or running, then returns its final status, error and domains with their route health.
- **`exec`** changes. `start` and `stop` run immediately. Everything else is a proposal a person confirms in Litepod: set or delete environment variables, redeploy, restart, roll back to an earlier deploy, edit the compose file or source, add, update or delete domains, update an application's settings, create, rename or delete projects and applications (empty or Compose).
  - **`deploy_static_site`** — one confirmation for a whole static site: project (existing or new), application, upload, deploy and an optional HTTPS domain. The agent gets a one-time `upload_url` and sends the site (up to 10 MB) with `curl -T site.zip <upload_url>`.
  - **`deploy_zip`** — one confirmation to build and run a project you have as files, not as a repository. The agent asks which folder of the ZIP holds the project and the port, gets a one-time `upload_url`, and sends the ZIP (up to 100 MB) with `curl -T project.zip <upload_url>`. It builds with Railpack, like a repository.
  - **`deploy_repository`** — one confirmation to build and run a Git repository, public or reachable through one of your Git providers: project (existing or new), application, build, deploy and an optional HTTPS domain.
  - **`create_from_template`** — one confirmation for a database (`postgres`, `mariadb`, `mongodb`, `dragonflydb`, `keydb`, `sqlserver`) or a catalog template such as `uptime-kuma`: project (existing or new), the variable values the user wrote, an optional domain, and the deploy.

  - **`deploy_compose`** — one confirmation for a Compose application from a file the agent writes: project (existing or new), the compose file, an optional `.env`, and the deploy.

  After confirming, the agent calls `wait_deployment` until the deploy finishes and reports back.

## Screenshots

<table>
  <tr>
    <td width="50%">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="assets/screenshots/overview-dark.webp">
        <img src="assets/screenshots/overview-light.webp" alt="Overview">
      </picture>
      <p align="center"><b>Overview</b> — status, deploy URL and compose in one view</p>
    </td>
    <td width="50%">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="assets/screenshots/deployments-dark.webp">
        <img src="assets/screenshots/deployments-light.webp" alt="Deployments">
      </picture>
      <p align="center"><b>Deployments</b> — every build with its live log</p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="assets/screenshots/analytics-dark.webp">
        <img src="assets/screenshots/analytics-light.webp" alt="Analytics">
      </picture>
      <p align="center"><b>Analytics</b> — requests, errors and latency through the proxy</p>
    </td>
    <td width="50%">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="assets/screenshots/metrics-dark.webp">
        <img src="assets/screenshots/metrics-light.webp" alt="Metrics">
      </picture>
      <p align="center"><b>Metrics</b> — CPU and memory per application</p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="assets/screenshots/terminal-dark.webp">
        <img src="assets/screenshots/terminal-light.webp" alt="Terminal">
      </picture>
      <p align="center"><b>Terminal</b> — a shell inside any container</p>
    </td>
    <td width="50%">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="assets/screenshots/volumes-dark.webp">
        <img src="assets/screenshots/volumes-light.webp" alt="Volumes">
      </picture>
      <p align="center"><b>Volumes</b> — persistent storage and live mounts</p>
    </td>
  </tr>
  <tr>
    <td width="50%">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="assets/screenshots/fleet-dark.webp">
        <img src="assets/screenshots/fleet-light.webp" alt="Fleet">
      </picture>
      <p align="center"><b>Fleet</b> — every server on one globe</p>
    </td>
    <td width="50%">
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="assets/screenshots/templates-dark.webp">
        <img src="assets/screenshots/templates-light.webp" alt="Templates">
      </picture>
      <p align="center"><b>Templates</b> — start from a known stack</p>
    </td>
  </tr>
</table>

## Get started

There are two ways to run Litepod. Pick the one that fits how much you want to operate yourself.

### Option 1 — Self-host

Install Litepod on any Linux server (Debian, Ubuntu, Fedora or Arch). The installer sets up Podman, generates every secret and starts the stack. It works as root or rootless:

```sh
curl -fsSL https://litepod.sh/install.sh | bash
```

Then open the panel, create your account and set your domain under **Settings → Server Domain**.

See the [installation docs](https://docs.litepod.sh) for update channels, backups, remote servers and resource sizing.

### Option 2 — Litepod Cloud

**Your server. Our control plane.**

Don't want to install, update and back up the control plane yourself? Use [Litepod Cloud](https://app.litepod.sh/register?utm_source=github&utm_medium=readme&utm_campaign=cloud). Register, connect a server you already pay for with one command, and deploy your first app in minutes.

- Managed control plane, updates and backups
- Your server, your data, your bill
- Move back to self-hosting whenever you want

## Try it first

Open the [live demo](https://demo.litepod.sh) and sign in with any username and password. It is a static demo, so it never touches a real server.

## Development

The code lives in [`apps/`](apps):

| Path | Stack |
|------|-------|
| [`apps/api`](apps/api) | Rust, axum, sqlx + SQLite, DuckDB (analytics and metrics, beta), podman-api |
| [`apps/ui`](apps/ui) | SvelteKit, Svelte 5, Tailwind CSS 4, shadcn-svelte |

Local setup, Podman machine sizing and staging deploys are documented in [`apps/README.md`](apps/README.md) and [`apps/INSTALLING.md`](apps/INSTALLING.md).

## Telemetry

Self-hosted Litepod sends an anonymous ping to our status service (`status.litepod.sh`) once a day, so we can count how many installations exist and which versions they run. The ping contains only a random instance ID (a UUID v7 generated on your server) and the Litepod version. It does not include domains, emails, app names, project data or IP addresses, and we do not store IPs.

It is on by default, and the first sysadmin to log in is told about it. To turn it off:

- In the UI: **System > Telemetry**.
- Or set `LITEPOD_TELEMETRY=off` in the environment, which overrides the setting.

Litepod Cloud and development builds never send it.

## Security

Found a vulnerability? Please follow [SECURITY.md](SECURITY.md) and do not open a public issue.

## Enterprise features

Code in `ee/` directories needs a [Litepod Enterprise subscription](LICENSE_EE.md) to run in production. It is for self-hosted instances only, and does not exist on Litepod Cloud. See [`apps/api/src/ee`](apps/api/src/ee/README.md) and [`apps/ui/src/lib/ee`](apps/ui/src/lib/ee/README.md) for the full list.

- **Single sign-on** — OIDC and SAML per organization, group-to-role mappings, verified domains and SCIM provisioning.
- **White-label** — set your own name, logo and favicon for the panel, the login page and the browser tab, from System → Branding.

## License

Litepod is licensed under the [Apache License 2.0](LICENSE), except for code in directories named `ee/`, which is covered by the [Litepod Enterprise License](LICENSE_EE.md).

<div align="center">
  <sub><a href="https://litepod.sh">litepod.sh</a></sub>
</div>
