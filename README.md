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

- **`get`** reads. Every project and application in the organization with its status (`running`, `stopped`, `failed`, …) in one call, or one project; every domain in the organization, or one application's; plus environment variable names (never values), deployments and their logs, live containers and their logs, source, compose file, servers and Git providers; the internal database templates and the template catalog, and for one template its ports and the variables the user has to fill in.
- **`exec`** changes. `start` and `stop` run immediately. Everything else is a proposal a person confirms in Litepod: set or delete environment variables, redeploy, edit the compose file or source, add, update or delete domains, create, rename or delete projects and applications (empty or Compose).
  - **`deploy_static_site`** — one confirmation for a whole static site: project (existing or new), application, upload, deploy and an optional HTTPS domain. The agent gets a one-time `upload_url` and sends the site (up to 10 MB) with `curl -T site.zip <upload_url>`.
  - **`create_from_template`** — one confirmation for a database (`postgres`, `mariadb`, `mongodb`, `dragonflydb`, `keydb`, `sqlserver`) or a catalog template such as `uptime-kuma`: project (existing or new), the variable values the user wrote, an optional domain, and the deploy.

  After confirming, the agent reads `get applications` until the status is `running` and reports back.

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

## Security

Found a vulnerability? Please follow [SECURITY.md](SECURITY.md) and do not open a public issue.

## License

Litepod is licensed under the [Apache License 2.0](LICENSE), except for code in directories named `ee/`, which is covered by the [Litepod Enterprise License](LICENSE_EE.md).

<div align="center">
  <sub><a href="https://litepod.sh">litepod.sh</a></sub>
</div>
