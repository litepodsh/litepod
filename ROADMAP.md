# Roadmap

**English** · [Español](ROADMAP.es.md)

Litepod is in **stability mode**. The current focus is making what already exists reliable,
not adding new features.

## Current focus

### 1. Stable deployments

Deployments should succeed or fail predictably, with clear logs and status.

- Fix deployments that hang, fail silently or end in the wrong status.
- Clean recovery after a failed or interrupted deploy (rollback, retries, leftover containers).
- Reliable builds and redeploys for every source type (Git, Compose, templates, static sites).
- More tests around the deploy pipeline.

### 2. Podman rootless

Rootless Podman should work as well as rootful, on every supported distro.

- Fix the differences between root and rootless installs (networking, ports, volumes, permissions).
- Make the installer and upgrades reliable in rootless mode.
- Document the limits of rootless and how to work around them.

### 3. MCP and agent assistance

The MCP endpoint lets an agent (Claude, Cursor, …) read and operate Litepod. The goal is for
it to be stable and trustworthy, not to add more tools.

- Make `get` and `exec` return consistent, accurate data.
- Make proposals (`exec` actions a person confirms) reliable from creation to deploy.
- Clear errors the agent can act on.
- More tests around the MCP tools.

## Feature requests

New features are not on the roadmap while the project is stabilizing. If you want one, open an
issue in this repository: [github.com/litepodsh/litepod/issues](https://github.com/litepodsh/litepod/issues).
Pick **Feature request** in the issue template. Requests are tracked and may be scheduled once
the project leaves stability mode. See [CONTRIBUTING.md](CONTRIBUTING.md) for the full flow.
