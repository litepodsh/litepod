# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Developers who self-host: independent developers and people on small teams who deploy their own apps, databases and services to servers they own.

They usually open the panel for a short visit with a specific reason: deploy something, check a failure, or change a setting. Then they leave. The panel has to orient them quickly, not hold them for a long session.

Teams, Litepod Cloud users and instance administrators also use the product, but they are secondary audiences.

## Product Purpose

Litepod is an open-source, self-hostable PaaS built on Podman, offered as a lightweight alternative to Vercel, Coolify and Dokploy. It deploys applications, databases and services to servers the user owns, through a clear panel rather than a black box.

Success means a developer can ship and keep running their apps on their own machines, and can tell at a glance whether everything is healthy and what to do when it is not.

## Positioning

Lightness and rootless operation are the main differentiator. Litepod uses daemonless, rootless Podman with no privileged socket exposed to the internet. Its control plane is a Rust API on SQLite, and it needs no Postgres or Redis. A fresh install runs in about 63 MB of RAM across three containers, so the platform does not need a server of its own.

## Operating Context

- Short, task-driven visits to the web panel, on servers the user controls or on Litepod Cloud.
- Work happens around Git pushes, `compose.yml` files, one-click templates, build and container logs, a web terminal, domains and backups.
- An agent (Claude, Cursor and others) can read and operate Litepod over MCP. Actions it sends through `exec` run immediately and return their result.

## Capabilities and Constraints

- **Capabilities:** Git deployments, Compose applications, one-click templates, databases, automatic HTTPS through Caddy, multi-server fleets over key-only SSH, request analytics and per-application metrics, web terminal, volumes, S3 backups, organizations with members and roles, and the MCP endpoint.
- **Roadmap constraint:** the project is in stability mode. The focus is stable deployments, rootless Podman and trustworthy MCP, not new features.
- **Interface languages:** English and Spanish.
- **Terminology:** organization, project, environment, application, deployment, container, domain, server.
- **Licensing:** open source under `LICENSE`, except directories named `ee/`, which use the Litepod Enterprise License.
- **Known preview data:** some surfaces the API cannot back yet, such as deploy steps and terminal sessions, render example data that is marked as such (`apps/ui/src/lib/preview/preview-data.ts`).

## Brand Commitments

- **Name:** Litepod, with the tagline "Host your apps. Keep control."
- **Logo:** `assets/litepod.svg`.
- **Binding visual constraints from the owner:** orange stays as the brand color. Gradients are allowed but must be subtle, not strongly marked.

## Evidence on Hand

- The 63 MB figure, measured with `podman stats` and stated in `README.md`.
- Product screenshots in light and dark themes in `assets/screenshots/`.
- Public surfaces: [website](https://litepod.sh), [docs](https://docs.litepod.sh), [live demo](https://demo.litepod.sh), [templates](https://templates.litepod.sh).
- No customer testimonials, case studies or press have been confirmed. Do not invent any.

## Product Principles

1. **Orient first.** Someone arriving for a short visit should know right away what is healthy, what failed and what to do next.
2. **Light is the promise.** The panel should earn the same reputation for lightness as the runtime it controls.
3. **Tell the truth about state.** Statuses, numbers and logs must match what the system is doing, and anything that is not real data must say so.
4. **The person keeps control.** Users own their servers and data, and decide which agents can operate them.
5. **Reliability before features.** While the project is in stability mode, making what exists dependable comes before adding more.
