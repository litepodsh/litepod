# Testing the staging stack

Staging bundles `api`+`ui` into one image (`Dockerfile`), served behind `caddy`, with SQLite embedded in the API. Compose file: `podman-compose-staging.yml`.

## 1. Build + run

```bash
sudo bash scripts/install-staging.sh
```

The first run creates `.env` and exits so its required values can be set. Re-run the command
after configuring it; it installs prerequisites if needed, enables the host Podman socket, then
builds and starts the stack.

To stop and remove the stack while retaining the database volume, run:

```bash
sudo bash scripts/uninstall-staging.sh
```

Add `--purge-data` only when all persistent staging data should be deleted.

Check everything is up:

```bash
podman compose -f podman-compose-staging.yml ps
podman compose -f podman-compose-staging.yml logs -f app
```

Stop / tear down:

```bash
podman compose -f podman-compose-staging.yml down
```

Rebuild after code changes (no cache):

```bash
podman compose -f podman-compose-staging.yml build --no-cache app
podman compose -f podman-compose-staging.yml up -d
```

## 2. Ports

- `caddy` → `http://localhost:80` (entrypoint for everything: UI + `/api/v1/*`)
- `database` → local SQLite file at `DATABASE_PATH` — inspect with `sqlite3` inside the API image
- `app` (api+ui) has **no published host port** — only reachable through `caddy` inside the compose network (`app:6001`)

## 3. Access via browser (UI)

Open `http://localhost/` — should load the SvelteKit UI (login page). Client-side routes (`/settings/organizations`, etc.) work via the SPA fallback.

## 4. Health check

```bash
curl http://localhost/health
```
Expect:
```json
{"status":"ok","database":"up","caddy":"up"}
```

## 5. Testing the API (curl or Postman)

Base URL: `http://localhost`

The API uses **cookie-based sessions** (`session_token` cookie set on register/login/setup), not bearer tokens. In Postman: enable "Automatically follow redirects" is irrelevant, but make sure **cookies are enabled** (Postman does this by default per-domain) — send requests to `http://localhost` so the cookie jar is shared across calls.

### First-run setup (creates the first sysadmin user)

```bash
curl -i http://localhost/api/v1/auth/setup
# {"needs_setup": true} if no users exist yet

curl -i -c cookies.txt -X POST http://localhost/api/v1/auth/setup \
  -H "Content-Type: application/json" \
  -d '{"name":"Admin","email":"admin@example.com","password":"admin1234"}'
```
This creates user #1, makes it sysadmin, creates the root organization, and sets the `session_token` cookie (saved to `cookies.txt` for curl's `-b` flag on later requests).

### Login (subsequent users / sessions)

```bash
curl -i -c cookies.txt -X POST http://localhost/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -d '{"email":"admin@example.com","password":"admin1234"}'
```

### Authenticated requests

Use `-b cookies.txt` to send the session cookie:

```bash
curl -b cookies.txt http://localhost/api/v1/auth/me
curl -b cookies.txt http://localhost/api/v1/users
curl -b cookies.txt http://localhost/api/v1/organizations
```

### Postman

1. Create a request `POST http://localhost/api/v1/auth/setup` (or `/login`) with a JSON body as above → Send. Postman stores the `session_token` cookie automatically for the `localhost` domain.
2. Subsequent requests to `http://localhost/api/v1/...` in the same Postman workspace reuse that cookie automatically — no manual header needed.
3. Full endpoint list + schemas: `http://localhost/docs` (Scalar/OpenAPI UI, served by the app itself).

### Logout

```bash
curl -i -b cookies.txt -X POST http://localhost/api/v1/auth/logout
```

## 6. Common issues

- **`/health` shows `"database":"down"`**: verify `DATABASE_PATH` is writable and that the API volume is mounted at `/var/lib/litepod` in containerized environments.
- **UI loads but API calls fail with CORS errors**: shouldn't happen in staging since UI and API share the same origin (`app` serves both behind `caddy`) — if you see CORS errors you're probably hitting the app directly on a different origin/port.
- **`setup` returns `needs_setup: false` unexpectedly**: a user already exists in the persistent data volume or local database file from a previous run. Either log in with existing credentials, or delete the local database file / `litepod_data` volume before restarting. There is no debug endpoint to reset the database from the UI/API.
