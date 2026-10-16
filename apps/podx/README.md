# podx/ — build, push, deploy litepod

Cross-platform release tooling. Targets `linux/amd64` (Ubuntu x86_64) from
any host. Produces one image with API + UI embedded.

## Where config lives

| File | Lives where | Purpose |
|---|---|---|
| `podx/.env`            | your dev machine | DockerHub and release-manifest publishing credentials |
| `api/.env`             | dev machine + each server | stable/alpha versions and runtime config litepod reads at boot |
| `apps/.env` (root)     | each server (written by `install-staging.sh`) | runtime config podman-compose injects into the container |
| `install-staging.sh`   | server (one-time) | bootstraps `apps/.env` with generated secrets + litepod image defaults |

Image/updater defaults (`docker.io/litepod/litepod`, stable channel, 3600s) are
hardcoded in `api/src/main.rs` — litepod works out of the box without any
of these env vars. Override only if you need to.

## One-time setup (dev machine)

```sh
# Linux native build is recommended — QEMU emulation on Mac arm64 can SIGSEGV
# rustc 1.97. On a Linux x86_64 host the build runs natively in ~5min.

cp podx/.env.example podx/.env
$EDITOR podx/.env   # set DOCKERHUB_TOKEN (PAT scoped to litepod/litepod)

DOCKERHUB_TOKEN=$(grep ^DOCKERHUB_TOKEN= podx/.env | cut -d= -f2-) \
  podman login docker.io -u litepod --password-stdin
```

## Release flow

```sh
just release-dockerhub
just release-dockerhub -- --alpha
just release-dockerhub -- --no-cache
just release-dockerhub -- --alpha --no-cache
```

The first command releases the stable channel from `LITEPOD_VERSION`; the second releases the
alpha channel from `LITEPOD_VERSION_ALPHA`. Increment the alpha counter in
`LITEPOD_VERSION_ALPHA` manually before each alpha release.

Each release pushes an immutable version tag and a channel tag to `docker.io/litepod/litepod`:

| Tag          | Purpose                              |
|--------------|--------------------------------------|
| `vX.Y.Z`     | Immutable, pinneable, traceable      |
| `latest`     | Floating stable channel               |
| `alpha`    | Floating alpha channel              |

Before pushing, the release checks whether the immutable version tag already exists. If it does,
the release reuses that Docker Hub artifact instead of overwriting it: it pulls the immutable tag
and uses the pulled image to update `latest` or `alpha`. An unavailable or unverifiable tag
aborts the release. Manifest publication uses conditional S3 writes and retries conflicts, so
concurrent stable and alpha publications cannot silently replace each other's channel entries.
`release-dockerhub` reuses the local Podman layer cache by default and verifies that the local OCI
version label matches the requested tag before contacting Docker Hub. Pass `-- --no-cache` only
when a clean rebuild is required; running `just clean` is not required before a release.
Every successful release also uploads the current lifecycle scripts with signed S3 PUTs:
`https://rustfs.sebasgc.xyz/litepod/install.sh` and
`https://rustfs.sebasgc.xyz/litepod/install-cloud.sh` and
`https://rustfs.sebasgc.xyz/litepod/uninstall.sh`. Either upload failing aborts the release
before the channel manifest is published. Before announcing either channel, the release also
publishes the app and core-service pins together in that channel's `version.json` object.
Stable uses `LITEPOD_VERSION` and `CADDY_VERSION`; alpha uses
`LITEPOD_VERSION_ALPHA` and `CADDY_VERSION_ALPHA`, all from `api/.env`.
Every service tag is required, must not be `latest`, and is validated against its registry.

## Deploy on QA server (Ubuntu x86_64)

```sh
# one-time: install + bootstrap apps/.env with secrets
scp -r apps ubuntu@<qa-ip>:/opt/litepod
ssh ubuntu@<qa-ip> "cd /opt/litepod/apps && sudo bash scripts/install-staging.sh"

# bring up QA
ssh ubuntu@<qa-ip> "cd /opt/litepod/apps && podman-compose -f podx/compose-qa.yml up -d"
```

## How the self-update works

The litepod API inside the container:

1. Reads `LITEPOD_UPDATE_CHANNEL` and the immutable tag used to create the running container.
2. Every `LITEPOD_UPDATE_INTERVAL` seconds (default 3600), fetches the release manifest and resolves
   only the selected stable or alpha channel.
3. Parses and compares the published and running versions. Fetch errors, missing channels, and
   invalid versions are reported as failed checks, never as “up to date.”
4. Reports an available update in System. Detection never installs automatically.
5. On an explicit install click, verifies the channel and target still match exactly what the UI
   showed, pulls that immutable tag, and starts a replacement with the captured ports, mounts,
   network, restart policy, and explicit environment.
6. Writes the selected tag/channel pin to `litepod_data` while the old process is still alive, then
   retires the old container. A persistence failure rolls the replacement back. Downtime is
   approximately 10 seconds.

## Inspect / force check

```sh
# current state (channel, current/latest version, check status, update_available)
curl https://qa/api/v1/system/version -H "Cookie: <session>"

# force an immediate check (does not wait for the timer)
curl -X POST https://qa/api/v1/system/update/check -H "Cookie: <session>"

# install exactly the target previously shown by the version endpoint
curl -X POST https://qa/api/v1/system/update/apply \
  -H "Cookie: <session>" -H "Content-Type: application/json" \
  -d '{"target_version":"v0.1.23-alpha.2","channel":"alpha"}'

# on the server: see the env vars the container actually has
podman inspect litepod-qa-api | jq '.[0].Config.Env' | grep LITEPOD_
```

## Disable

```sh
# on the server: set LITEPOD_UPDATE_INTERVAL=0 in apps/.env, then
ssh ubuntu@<qa-ip> "cd /opt/litepod/apps && podman-compose -f podx/compose-qa.yml up -d"
```
