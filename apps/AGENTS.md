# AGENTS.md

## Stack
- `ui/` - SvelteKit + Svelte 5, Tailwind CSS 4, shadcn-svelte components (style `luma`, icons `lucide`)
- `api/` - Rust + axum, sqlx + SQLite, podman-api crate
- Local services (caddy) run via Podman Compose (`podman-compose.yml`)

## Package manager
- **Always use `bun`** in `ui/`. Never `npm install` / `npm run` / `yarn`.
  - `bun install`, `bun run dev`, `bun run build`, `bun run check`
- Bun only. Never pnpm/npm/yarn, even as a fallback - `bun.lock` is the only lockfile.

## Running things
Use the root `justfile` - don't call `bun`/`cargo`/`podman` directly for dev workflows.
- `just ui` - ui dev server only
- `just api` - api dev server only (cargo watch)
- `just dev` - ui + api together
- `just compose` - start podman compose services (caddy)
- `just compose-down` - stop podman compose services

## Containers
- Podman only, not Docker (`podman compose`, `podman-api` crate in `api/`).
- Compose file: `podman-compose.yml`. Services: `caddy` (80/443).

## UI conventions
- Components: shadcn-svelte, aliases in `ui/components.json` (`$lib/components/ui`, `$lib/components`, `$lib/hooks`, `$lib/utils`).
- Add new shadcn components with the shadcn-svelte CLI (via `bunx`), not hand-written from scratch, unless customizing an existing one.
- Icons: `@lucide/svelte`.
- Svelte 5 runes syntax (`$state`, `$derived`, `$effect`), not legacy Svelte 4 stores/reactive statements, in new code.

## API conventions
- Rust edition 2024, axum web framework.
- Run/watch with `just api` (`cargo watch -x run`), not raw `cargo run`, during dev.
- Migrations (`api/migrations/*.sql`) are append-only: never edit a file after it has shipped, always add a new numbered one. Enforced at boot by a checksum check (`migrations/mod.rs`) that panics on a mismatch.
- Never add a `UNIQUE` column to an existing table with inline syntax — SQLite has never allowed `UNIQUE` in `ALTER TABLE ADD COLUMN`. Always two statements: `ALTER TABLE x ADD COLUMN y TEXT;` then `CREATE UNIQUE INDEX idx_x_y ON x(y);`. A brand-new table may still declare `UNIQUE` inline in its own `CREATE TABLE`.
