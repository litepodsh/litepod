# Installing litepod

This covers first-time setup on a new machine, including Podman resource sizing —
skip to [Resource sizing](#resource-sizing) if Podman is already installed.

## Linux update channels

Use the default stable channel for production releases, or choose alpha for builds published
ahead of stable:

```bash
curl -fsSL https://litepod.sh/install.sh | bash
curl -fsSL https://litepod.sh/install.sh | bash -s -- --alpha
```

The corresponding removal script is published at
`https://litepod.sh/uninstall.sh`.

Re-running the installer in the same channel repairs the stack while preserving its `.env`,
database, and volumes. Changing channels is blocked to preserve data; use a new host instead, or
pass `--wipe` only when you intend to delete the existing installation after its confirmation.

## macOS / Windows (local development, via Podman Desktop)

1. Install [Podman Desktop](https://podman-desktop.io/). On first launch it creates a
   Podman machine — a Linux VM that actually runs your containers, since Podman has no
   native macOS/Windows runtime.
2. Size the machine before starting the stack (see [Resource sizing](#resource-sizing)
   below). Undersized machines are the main cause of Railpack build failures/timeouts.
3. From `apps/`, follow the [Quick Start](README.md#quick-start-local-development) in
   the README.

## Linux (staging / server)

Native Podman on Linux runs directly on the host — there is no VM, so there is nothing
to size beyond the host itself meeting the recommendations below.

```sh
git clone <repository-url> litepod
cd litepod/apps
sudo bash scripts/install-staging.sh
```

The installer prints detected CPU/RAM and, on small hosts, recommends a
`Max concurrent builds` value for Settings → Build Configuration. See
[README.md](README.md#staging--server-deploy-linux-only) for the full walkthrough.

## Resource sizing

Each Railpack deployment runs its BuildKit worker inside a temporary, isolated
rootless-Podman build container. It has no application-data mount. BuildKit
uses the namespace-scoped privileges needed for nested build cgroups; configure
optional CPU and RAM caps per application in **Advanced → Build resources**.
Those caps apply to that application's Docker or Railpack builds. Concurrent
builds still need enough aggregate host capacity.

| Setup | Minimum | Recommended | Notes |
|---|---|---|---|
| Single build at a time | 2 CPU / 4 GiB | 4 CPU / 4 GiB | Fine for sequential deploys |
| 2+ concurrent Railpack builds | 4 CPU / 4 GiB | **8 CPU / 8 GiB** | What this project's queue defaults to (`max_concurrent_builds = 3`) |

If you can't give the machine/host that much, lower `Max concurrent builds` in
Settings → Build Configuration (down to `1` serializes all builds, which is always
safe — just slower) instead of leaving the default and hitting stalls.

### Resizing the Podman machine (macOS/Windows)

**GUI (Podman Desktop):** Settings (gear icon, bottom-left) → **Resources** tab → find
your machine card → adjust the CPU/Memory sliders → **Update**. Podman Desktop stops
and restarts the machine to apply the change — anything running in it stops briefly.

**CLI:**

```sh
podman machine stop
podman machine set --cpus 8 --memory 8192
podman machine start
```

After a resize, start litepod's infrastructure stack again:

```sh
cd apps
just compose
```

New litepod static, Dockerfile, and Railpack deployments use Podman's
`unless-stopped` restart policy. Once Podman is available again, applications
that were not explicitly stopped return automatically; an application stopped
by the user remains stopped. Existing applications receive this policy on
their next redeploy. Compose applications keep the `restart:` policy declared
in their own Compose YAML.

### Linux

No VM step — just make sure the host itself meets the table above.
`scripts/install-staging.sh` checks this automatically and warns if the host is
undersized. It enables `podman-restart.service` for rootful Podman. Rootless
installs also enable a dedicated `litepod-stack.service` user unit that brings
the complete litepod Compose stack up after a server reboot.

Verify the service with the command matching that scope:

```sh
# Rootful
systemctl is-enabled podman-restart.service

# Rootless (replace deploy with LITEPOD_PODMAN_USER)
deploy_uid=$(id -u deploy)
sudo -u deploy env \
  XDG_RUNTIME_DIR="/run/user/$deploy_uid" \
  DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$deploy_uid/bus" \
  systemctl --user is-enabled podman.socket litepod-stack.service
```

Both rootless units must report `enabled`; rerun `scripts/install.sh` as the
same installation user to repair them.

## Preparing a remote application server

Remote servers must be directly reachable by SSH from the litepod host, running
Debian, Ubuntu, Fedora, or Arch Linux. Use a dedicated unprivileged deployment user for rootless
Podman. Add the key litepod generates in **Servers** to that user's
`~/.ssh/authorized_keys`; use `chmod 700 ~/.ssh` and `chmod 600
~/.ssh/authorized_keys`. litepod verifies the server's ED25519 fingerprint before
the first command and refuses a changed key until a sysadmin confirms it.

Setup needs non-interactive `sudo` only to install/upgrade Podman prerequisites,
configure subordinate IDs and linger, enable the Podman user socket, and—on a
public-edge server—install/reload the narrow root-owned Caddy route helper. It
does not grant litepod a general remote shell or expose the Podman API over TCP.
Setup also installs `podman-compose` and verifies the selected user's
`podman compose version`; rerun Repair after upgrading an already-registered
server that reports a missing Compose provider.

For **public edge**, configure DNS directly to the remote public IP/hostname
and allow inbound TCP 80/443; native Caddy on that host obtains certificates.
For **private relay**, leave DNS on litepod's central public address and do not
open remote HTTP/S ports: traffic uses a supervised key-only SSH forward to an
application loopback port. Public edge is the lower-overhead choice when a
server can safely receive Internet traffic; private relay is for LAN-only
servers. A **private-only** server cannot receive public domains.

After reboot, litepod reconciles remote Caddy fragments and relays for running
applications. If a route is unhealthy, run Health check then Repair from
Servers. Repair validates Caddy before reload and preserves the old route on
failure. Check the application's loopback listener with a redeploy/start when
the server is healthy but a relay cannot be recreated.

## Lost your sysadmin password?

Self-hosted installs configure `RESEND_API_KEY` as optional in
`scripts/install.sh`, so the "Forgot password" link on the login page silently
succeeds without sending an email when the variable is unset. The same applies
when the original inbox is gone. litepod ships an operator CLI inside the
`litepod-api` container that talks directly to the local SQLite database
(`/var/lib/litepod/litepod.db`):

```bash
# See who the sysadmins are (id, email, name, 2FA status, created_at).
podman exec litepod-api litepod-api list-sysadmins

# Reset a sysadmin's password. The new password is prompted; sessions are
# revoked, so the operator signs in again immediately.
podman exec -it litepod-api litepod-api reset-password --email admin@example.com

# No TTY? Print a strong random password on stdout.
podman exec litepod-api litepod-api reset-password --email admin@example.com --generate

# Lost access to the authenticator app? Clear 2FA so the next login skips it.
podman exec -it litepod-api litepod-api reset-2fa --email admin@example.com

# The original inbox is dead? Move the account to a new email.
podman exec -it litepod-api litepod-api change-email \
    --email admin@example.com --new-email admin@newdomain.test
```

All four subcommands share the same fail-fast checks as the API process: the
binary refuses to run unless `HASH_SECRET` and `ENCRYPTION_KEY` are set in the
container environment (which they always are on a standard install). The
underlying database update is one transaction per command, so a partially-run
recovery is not possible.

To rehearse the recovery path against a local dev database without touching a
running install, use the `just ops-dev` recipe from the repository root:

```bash
just ops-dev list-sysadmins
just ops-dev reset-password --email admin@example.com --generate
```
