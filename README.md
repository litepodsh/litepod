# Apps

Local development environment with Podman Compose.

## Services

| Service | Port | Description |
|---------|------|-------------|
| SQLite (sqlx) | - | Local database file owned by API |
| Caddy   | 80   | Reverse proxy (localhost) |

## Prerequisites

- [Podman](https://podman.io/) installed
- `podman-compose` installed

First time setting up Podman, or deploying to a new machine? See
[INSTALLING.md](INSTALLING.md) — it covers Podman machine setup on macOS/Windows and
the CPU/RAM sizing that concurrent Railpack builds need (undersized machines cause
builds to stall or exhaust the available host resources).

## Quick Start (local development)

Use this for everyday development on your machine:

```sh
podman compose -f podman-compose.yml up -d
```

### Local static deployments on macOS

litepod runs locally on macOS while Podman runs inside its Linux machine, so static deployment
artifacts must use a shared host directory rather than the staging named volume. Before
starting `just api`, export these values from `apps/`:

```sh
export LITEPOD_DATA_DIR="$PWD/.litepod-data"
export LITEPOD_HOST_DATA_DIR="$PWD/.litepod-data"
export LITEPOD_DATA_VOLUME=""
export PODMAN_SOCKET="unix://$(podman machine inspect --format '{{.ConnectionInfo.PodmanSocket.Path}}')"
```

`$PWD` is under `/Users`, which Podman Desktop's machine shares with macOS. litepod creates the
directory on first upload. Staging uses the Compose-managed `litepod_data` named volume instead.

**Size the Podman machine before deploying anything through Railpack.** All Railpack builds
share one BuildKit container; on a small machine, two concurrent builds can time out and crash
it instead of queueing. See [Resource sizing](INSTALLING.md#resource-sizing) in INSTALLING.md —
8 CPU / 8 GiB is the recommended default (`podman machine set --cpus 8 --memory 8192`, or via
Podman Desktop → Settings → Resources).

## Production Deploy (Linux only)

One-liner, no clone needed — resolves the selected release channel to an immutable version tag,
pulls that image, and starts Caddy + Dragonfly + the app via Podman Compose. One script, two flows:

**Rootless (default, recommended).** SSH in as your normal user and run it with no
`sudo` prefix — it sets up rootless Podman for yourself and only shells out to `sudo`
internally for the few steps that need root (package install, enabling linger).
Installs under `~/litepod`:

The installer enables a `litepod-stack.service` systemd user unit, so the full litepod
stack starts automatically after a server reboot without requiring an SSH login for
that user.

```sh
curl -fsSL https://rustfs.sebasgc.xyz/litepod/install.sh | bash
```

### Choose an update channel

Run the installer with no flag for the stable channel, or pass `--alpha` for the alpha
channel:

```bash
curl -fsSL https://rustfs.sebasgc.xyz/litepod/install.sh | bash
curl -fsSL https://rustfs.sebasgc.xyz/litepod/install.sh | bash -s -- --alpha
```

Changing channels reruns the installer and permanently deletes all litepod data: Podman volumes,
the database, deployed applications, secrets, domains, and certificates. No backup is made, so
make a backup before confirming the required cleanup.

**Root (`sudo`/cloud-init).** Run it as root and it prompts to choose rootless
(default — targets `$SUDO_USER`, or `LITEPOD_PODMAN_USER`) or rootful. Installs under
`/opt/litepod`:

```sh
curl -fsSL https://rustfs.sebasgc.xyz/litepod/install.sh | sudo bash
```

Non-interactive root runs (no tty) default to rootless for `$SUDO_USER` too; skip the
prompt explicitly with `LITEPOD_MODE=rootless` or `LITEPOD_MODE=rootful`:

```sh
curl -fsSL https://rustfs.sebasgc.xyz/litepod/install.sh | sudo LITEPOD_MODE=rootful bash
```

Same script, run locally instead of piped:

```sh
bash scripts/install.sh          # rootless, as yourself
sudo bash scripts/install.sh     # prompts rootless/rootful
```

### Cloud edition (remote-only deployments)

Install the cloud edition on a new Linux host when litepod must manage only remote
servers. Its control plane still uses litepod's built-in version updater, but applications,
databases, and templates can be created only after a remote server has been connected and
reaches **Ready**.

```sh
curl -fsSL https://rustfs.sebasgc.xyz/litepod/install-cloud.sh | bash
# or from a checkout
bash scripts/install-cloud.sh
```

The cloud installer writes `IS_CLOUD=true` and refuses to run over a standard installation.
Cloud and standard installations cannot be converted; install cloud on a new host instead.

The matching published uninstaller is available at
`https://rustfs.sebasgc.xyz/litepod/uninstall.sh`:

```sh
curl -fsSL https://rustfs.sebasgc.xyz/litepod/uninstall.sh | bash
```

It installs Podman/`podman-compose`/openssl if missing, generates `.env` with every
secret auto-generated and the host's public IP auto-detected, resolves stable (or `--alpha`)
from `https://rustfs.sebasgc.xyz/litepod/version.json`, pins the resulting immutable tag, and
starts the stack. Override the repository with `LITEPOD_IMAGE_REPO` and use
`LITEPOD_VERSION_MANIFEST_URL` for a compatible custom manifest; the production installer does
not accept a floating `LITEPOD_IMAGE_TAG` override. After it's up, log in and set the real domain
under Settings → Server Domain — the auto-detected IP is only a bootstrap fallback for GitHub
App callbacks/webhooks until then.

The installer also configures Docker Hub (`docker.io`) as Podman's default registry
for unqualified image references. This applies to rootless and rootful deployments,
remains in place when litepod self-updates, and lets Dockerfiles use references such as
`FROM oven/bun:1`. Use fully qualified references for images hosted outside Docker Hub.

To inspect the rootless boot unit as the installation user, run
`systemctl --user status litepod-stack.service`. From root, use
`sudo -u <user> env XDG_RUNTIME_DIR="/run/user/$(id -u <user>)" DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u <user>)/bus" systemctl --user status litepod-stack.service`.

## Staging / Server Deploy (Linux only)

## Remote Podman servers

litepod can deploy an application to a directly SSH-reachable Debian, Ubuntu,
Fedora, or Arch Linux server. Register servers from **Servers** as a sysadmin; every new application
then explicitly selects one Ready server. Connections are key-only and litepod
never opens a Podman TCP socket or stores a plaintext private key.

1. Use the generated Ed25519 public key from the registration dialog, or create
   one yourself with `ssh-keygen -t ed25519 -f ~/.ssh/litepod_remote`.
2. On the remote host, install the public key in the deployment user's
   `~/.ssh/authorized_keys` (directory mode `700`, file mode `600`).
3. Run Preflight, compare and explicitly confirm the shown ED25519 host-key
   fingerprint, then run Setup. A changed fingerprint stops all SSH work until
   it is reviewed and confirmed again.

Rootless Podman under an unprivileged SSH user is the default. Setup enables
linger and the user's Podman socket; its narrow passwordless-sudo work is
limited to package installation (via `apt`, `dnf`, or `pacman`), subordinate-ID/linger setup,
and, for a public edge, installation and reload of litepod's native-Caddy route
helper. Setup also installs and verifies the `podman-compose` provider; a
Compose deployment cannot start until `podman compose version` succeeds for
the configured SSH user. Choose rootful only where the workload requires it.

Traffic mode is selected when registering the server:

- **Public edge:** DNS `A`/`AAAA` records point directly to the slave's public
  IP/hostname. Open TCP 80 and 443 on that server. Its native Caddy owns TLS
  and proxies only to the application loopback port; do not expose Podman app
  ports.
- **Private relay:** DNS remains pointed at litepod's central public IP. The
  slave needs no inbound HTTP/S firewall rule; central Caddy reaches it through
  a key-only SSH forward. This adds one encrypted hop and is best for LAN-only
  servers, while public edge avoids central bandwidth and relay dependency.
- **Private only:** public domains are intentionally unavailable.

If a remote server becomes degraded, use Health check and then Repair from the
Servers page. Repair is idempotent. On litepod restart, running public-edge
routes and private SSH relays are rebuilt from persisted applications, domains,
and endpoints. For a public-edge Caddy failure, run Repair; it validates the
root-owned route fragments and reloads Caddy with rollback on invalid config.
For a private relay failure, Health check the server and restart/redeploy the
application if its loopback listener is absent; litepod recreates the forwarding
process and central route automatically when it is healthy.

> ⚠️ **The staging stack (`podman-compose-staging.yml`) is designed for Linux servers only.**
> It mounts the selected host Podman API socket into the app container, which does **not** exist
> on macOS or Windows. Running it on non-Linux platforms will result in a `502 Bad Gateway`
> error from the app.

From a cloned repository, run the installer as root:

```sh
git clone <repository-url> litepod
cd litepod/apps
sudo bash scripts/install-staging.sh
```

The script prints detected host CPU/RAM and, on small hosts, recommends a `Max concurrent
builds` value for Settings → Build Configuration (see
[Resource sizing](INSTALLING.md#resource-sizing)). It then installs git/openssl/Podman/
`podman-compose` when needed, enables a rootless Podman socket for the invoking sudo user,
and creates `.env` from the safe template with every secret auto-generated (JWT signing
key/token, hash secret, encryption key) and the host's public IP auto-detected — no manual
edit needed on a normal run. It builds and starts the stack in that same run.
The script reports the selected Podman mode and socket before deployment. To target a different
rootless user, set `LITEPOD_PODMAN_USER`; when no user is selected, it falls back to rootful Podman:

```sh
sudo LITEPOD_PODMAN_USER=deploy bash scripts/install-staging.sh
```

To remove the staging containers while keeping database data for a later reinstall:

```sh
sudo bash scripts/uninstall-staging.sh
```

Pass `--purge-data` only when the Litepod data and Caddy named volumes should be permanently deleted.

After the stack is up, log in and set the real domain under Settings → Server Domain — the
auto-detected IP in `.env` is only a bootstrap fallback for GitHub App callbacks/webhooks
until a domain is configured there, which then takes over.

## Quick Start

1. **Configure environment**

   Edit `.env` and set application secrets. The embedded database file uses
   `DATABASE_PATH` and needs no database JWT.

   ```env
    DATABASE_PATH=./data/litepod.db
   ```

2. **Start services**

   ```sh
   podman compose up -d
   #or
   podman compose -f podman-compose.yml up -d
   ```

3. **Verify**

   ```sh
   # Check running containers
   podman compose ps

    # Test API and embedded database
    curl http://localhost:6001/health

   # Test Caddy proxy
   curl http://localhost/api/v1
   ```

4. **View logs**

   ```sh
   podman compose logs -f
   ```

5. **Stop services**

   ```sh
   podman compose down
   ```

## SQLite CLI

Runtime image includes `sqlite3` for local inspection:

```sh
sqlite3 /var/lib/litepod/litepod.db
```

## Volumes

Data persists across restarts in named volumes:

- `litepod_data` — embedded database and Litepod data
- `caddy_data` — TLS certificates
- `caddy_config` — Caddy config cache

Clear all data:

```sh
podman compose down -v
```
