# Domain Path Routing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let one public host send `/` to one target and `/api` (or any prefix) to another, by giving an application domain an optional `path`.

**Architecture:** A domain row gets a nullable `path`. A domain without a path is the host's *root* and keeps owning everything host-level in Caddy (listener, SNI, `automatic_https.skip`, the TLS automation policy, the certificate). A path domain adds only one Caddy route, `host + path` with the prefix stripped, and copies the root's `https`. Path routes are kept ahead of host-only routes in `srv0.routes`, longest path first. v1 runs only on the central Caddy (`application_uses_local_caddy`). Edge servers write one Caddyfile site block per host, so a second route on the same host would make Caddy reject the config as ambiguous.

**Tech Stack:** Rust (axum, sqlx/SQLite, Caddy admin JSON), SvelteKit + Svelte 5, vitest.

**Spec:** No separate design document. The requirements below were agreed in chat on 2026-11-05.
- Optional `path` per domain, such as `/api`. One host can serve `/` from app A and `/api` from app B, or from another Compose service of the same app.
- Caddy matches host and path. Longer paths win. The prefix is always stripped before proxying (no "keep prefix" option in v1).
- A host can have only one domain per path (`/api` once, root once).
- Certificates and DNS stay per host and belong to the root domain.

## Global Constraints

- Migrations are append-only. Add `apps/api/migrations/0088_application_domain_path.sql` and never edit an existing migration.
- New SQLite indexes are separate statements. Never put `UNIQUE` inline in `ALTER TABLE`.
- Use `bun` only in `apps/ui`. Use Svelte 5 runes. Relative UI imports have no extension, `#lib/...` imports end in `.js`, and files are `.ts`.
- Run formatters only on lines you wrote. Do not run `cargo fmt` on the whole crate or oxfmt on existing Svelte files.
- Rust `mod.rs` files only re-export. Logic goes in the named files listed below.
- UI copy ships in English (`en/projects.toml`) and neutral Spanish (`es/projects.toml`): no voseo, no regionalisms.
- Do not commit unless the owner asks.

## Review Focus

1. **Deleting or renaming a root that still has path domains.** The user expects to be told to remove `/api` first, not to see the root vanish while `/api` keeps a route with no TLS policy. Test: `root_with_paths_cannot_be_deleted_or_renamed` (Task 1).
2. **Removing a path domain while its root stays.** `https://host/` must keep working, so its SNI policy, `:443` and TLS automation policy must survive. Test: `removing_a_path_route_keeps_the_hosts_tls` (Task 2).
3. **Root re-pushed after a path domain exists** (redeploy, basic-auth change, restore). `/api` must still reach its own target and not be swallowed by `/`. Test: `path_routes_stay_ahead_of_the_root_after_a_repush` (Task 2).
4. **`/api` and `/api/`, `/API`, `/api//x`, `/../x`, `/`.** Trailing slashes are trimmed, case is kept, empty and `..` segments are rejected, and `/` means no path. Test: `normalize_path_cases` (Task 1).
5. **Turning HTTPS on or off for the root.** The children's stored `https`, and so their `url`, must follow. Test: `root_https_change_propagates_to_paths` (Task 1).

---

### Task 1: Store and validate `path`

**Files:**
- Create: `apps/api/migrations/0088_application_domain_path.sql`
- Modify: `apps/api/src/dtos/deployment.rs` (`DomainDto`, `CreateDomainRequest`, `UpdateDomainRequest`, `ExpectedRouteDto`)
- Modify: `apps/api/src/deployment/domains/validate.rs`
- Modify: `apps/api/src/deployment/domains/crud.rs` (`SELECT_COLUMNS`, `row_to_dto`, `create_with_origin_locked`, `update`, `delete`)
- Modify: every `CreateDomainRequest {` / `UpdateDomainRequest {` literal (`assistant/actions/apply.rs`, `deployment/domains/crud.rs:795`, `deployment/domains/tests.rs`): add `path: None`
- Test: `apps/api/src/deployment/domains/tests.rs`

**Interfaces:**
- Produces:
  - `DomainDto.path: Option<String>` and `ExpectedRouteDto.path: Option<String>`.
  - `CreateDomainRequest.path: Option<String>` and `UpdateDomainRequest.path: Option<String>`, both `#[serde(default)]`. For updates, `None` means "leave as is", and `Some("")` or `Some("/")` means "clear" (it normalizes to `None`).
  - `pub(super) fn normalize_path(raw: Option<&str>) -> Result<Option<String>, AppError>` in `validate.rs`.
  - `pub(super) async fn host_root(state: &SharedState, server_id: &str, host: &str) -> Result<Option<DomainDto>, AppError>` in `validate.rs`: the domain on that server with this host and `path IS NULL`.
  - `pub(super) async fn reject_orphaning_paths(state: &SharedState, server_id: &str, host: &str) -> Result<(), AppError>` in `validate.rs`.
  - `reject_conflicting_public_host(state, server_id, host, port, path: Option<&str>, exclude_domain_id)`: same host and same path is a conflict on any port. Same host with a different path is fine.

- [ ] **Step 1: Write the failing tests** in `domains/tests.rs`. Use `test_state(false)` and `create` as the existing tests do.

```rust
#[test]
fn normalize_path_cases() {
    assert_eq!(normalize_path(None).unwrap(), None);
    assert_eq!(normalize_path(Some("")).unwrap(), None);
    assert_eq!(normalize_path(Some(" / ")).unwrap(), None);
    assert_eq!(normalize_path(Some("/api/")).unwrap().as_deref(), Some("/api"));
    assert_eq!(normalize_path(Some("/API/v1")).unwrap().as_deref(), Some("/API/v1"));
    for bad in ["api", "/api//x", "/../x", "/a/./b", "/a b", "/a*", "/a?x", &format!("/{}", "a".repeat(200))] {
        assert!(normalize_path(Some(bad)).is_err(), "{bad} must be rejected");
    }
}

#[tokio::test]
async fn path_domain_needs_a_root_and_copies_its_https() { /* create app-1 "/api" on a.example.com
   before any root -> ValidationError containing "a.example.com"; create root (https: Some(true)), then
   app-2 path "/api" with https: Some(false) -> Ok, saved.https == true, saved.path == Some("/api"),
   saved.url == "https://a.example.com/api" */ }

#[tokio::test]
async fn same_host_and_path_is_a_conflict_on_any_port() { /* root a.example.com:80; "/api":3000 ok;
   "/api":4000 -> Err; "/docs":3000 -> Ok */ }

#[tokio::test]
async fn paths_are_rejected_for_local_hosts() { /* host "localhost", path "/api" -> ValidationError */ }

#[tokio::test]
async fn root_with_paths_cannot_be_deleted_or_renamed() { /* root + "/api"; delete(root) -> Err
   naming "/api"; update root host -> Err; delete "/api" then delete root -> Ok */ }

#[tokio::test]
async fn root_https_change_propagates_to_paths() { /* root http + "/api"; update root https=true;
   get("/api").https == true and url starts with "https://" */ }
```

- [ ] **Step 2: Run them and confirm they fail.**
Run: `cd apps/api && cargo test --lib deployment::domains::tests:: -- path normalize`
Expected: they do not compile, because `normalize_path` and `path` do not exist yet.

- [ ] **Step 3: Write the migration** `0088_application_domain_path.sql`:

```sql
ALTER TABLE application_domains ADD COLUMN path TEXT;
DROP INDEX idx_application_domains_host;
CREATE UNIQUE INDEX idx_application_domains_host ON application_domains(host, port, COALESCE(path, ''));
```

- [ ] **Step 4: Implement `normalize_path`.** Trim the input. Empty or `/` returns `None`. Otherwise it must start with `/`. Strip one trailing `/`. Each segment must be non-empty, not `.` or `..`, and contain only `[A-Za-z0-9._~-]`. The total length must be ≤ 200. The error text is `"Path must look like /api: letters, digits, - . _ ~ separated by /"`.

- [ ] **Step 5: Wire `path` through `crud.rs`.**
  - Add `path` to `SELECT_COLUMNS`, the `INSERT`, and the `UPDATE`.
  - `row_to_dto` sets `url = domain_url(..) + path.unwrap_or("")`.
  - In `create_with_origin_locked` and `update`, after `validate(..)`, when the normalized path is `Some`, reject in this order:
    1. Local host: `"Paths are not available for local hosts"`.
    2. `!application_uses_local_caddy`: `"Path routing is only available on the main server for now"`.
    3. `host_root` returns `None`: `"Add {host} without a path first; path domains use its HTTPS settings"`.
  - Then take `https` from the root and ignore the request's certificate fields by passing `CertificateRequest::default()` to `prepare_for_state`.
  - `delete`, and an `update` that changes a root's host or gives a root a path, call `reject_orphaning_paths(before.host)` first. Its error text is `"Remove the path domains of {host} first: {comma-separated paths}"`.
  - An `update` that changes a root's `https` runs `UPDATE application_domains SET https = ?1 WHERE host = ?2 AND path IS NOT NULL` in the same transaction as the root update.

- [ ] **Step 6: Run the tests and confirm they pass.**
Run: `cd apps/api && cargo test --lib deployment::domains`
Expected: PASS, including every existing domain test.

### Task 2: Route path domains in Caddy

**Files:**
- Modify: `apps/api/src/deployment/operations/caddy.rs` (`caddy_route`, `caddy_application_routes`, the restore loop near line 282, `route_present` callers, `apply_app_domain_to_config`, `apply_app_domain_removal_to_config`)
- Modify: `apps/api/src/deployment/domains/caddy_sync.rs` (`push_caddy_route`, the reconcile call at ~233, and `expected_routes`, which fills `ExpectedRouteDto.path`)
- Modify: `apps/api/src/servers/ingress.rs:393` (pass `domain.path.as_deref()`)
- Test: `apps/api/src/deployment/operations/tests.rs`

**Interfaces:**
- Consumes: `DomainDto.path` and `ExpectedRouteDto.path` from Task 1.
- Produces:
  - `apply_app_domain_to_config(config, id, hostname, path: Option<&str>, container_name, container_port, https, basicauth, maintenance)`: `path` is inserted after `hostname`.
  - `caddy_route(state, id, hostname, path: Option<&str>, container_name, container_port, https)`.
  - `fn sort_path_routes_first(routes: &mut [serde_json::Value])`.

- [ ] **Step 1: Write the failing tests** in `operations/tests.rs`, following the existing `apply_app_domain_to_config` tests (build `config`, apply, then inspect `srv0`):

```rust
#[test]
fn a_path_route_matches_host_and_prefix_and_strips_it() {
    // apply root "a.example.com" (path None, https true), then "litepod-route-api" with Some("/api")
    let api = /* route with @id litepod-route-api */;
    assert_eq!(api["match"][0]["host"], json!(["a.example.com"]));
    assert_eq!(api["match"][0]["path"], json!(["/api", "/api/*"]));
    assert_eq!(api["handle"][0], json!({ "handler": "rewrite", "strip_path_prefix": "/api" }));
    assert_eq!(api["handle"][1]["handler"], "reverse_proxy");
    // only the root added an SNI policy and a TLS automation policy
    assert_eq!(sni_policies_for("a.example.com").len(), 1);
    assert!(automation_policy_ids().iter().all(|id| id != "litepod-route-api-tls"));
}

#[test]
fn path_routes_stay_ahead_of_the_root_after_a_repush() {
    // apply root, "/api", "/api/v2", then root again
    assert_eq!(route_ids_in_order()[..3], ["litepod-route-v2", "litepod-route-api", "litepod-route-root"]);
}

#[test]
fn removing_a_path_route_keeps_the_hosts_tls() {
    // apply root (https) + "/api"; apply_app_domain_removal_to_config(config, "litepod-route-api", "a.example.com")
    // -> api route gone; root route, ":443", SNI "a.example.com", "litepod-route-root-tls" all still present
}

#[test]
fn basic_auth_runs_before_the_prefix_strip() {
    // path route with Some(&auth): handle[0].handler == "authentication", [1] == rewrite, [2] == reverse_proxy
}
```

- [ ] **Step 2: Run them and confirm they fail.**
Run: `cd apps/api && cargo test --lib deployment::operations::tests`
Expected: they do not compile, because `apply_app_domain_to_config` does not take `path` yet.

- [ ] **Step 3: Change `apply_app_domain_to_config`.**
  - With `Some(path)`, the matcher is `{ "host": [hostname], "path": [path, format!("{path}/*")] }`. Insert a `rewrite` handler with `strip_path_prefix` after the optional `authentication` handler and before `reverse_proxy`. Maintenance handles stay as they are.
  - Skip the maintenance `errors` route for path domains. Add a comment: `shortcut: no companion-death fallback on path routes, add a path matcher to errors_route if users hit it`.
  - Return right after inserting the route, so listen, SNI, `automatic_https` and TLS automation stay owned by the root.
  - Call `sort_path_routes_first(routes)` after every insert. It is a stable sort keyed by `Reverse(longest path in match[*].path, 0 if none)`, so routes without a path keep their relative order. Nothing else in this config uses `path` matchers today, so the bootstrap catch-all stays last.

- [ ] **Step 4: Change `apply_app_domain_removal_to_config`.** Remove the route and errors route by id as it does now. If any remaining `srv0` route still matches `hostname`, return before touching SNI, `skip`, the automation policy and `certificates::remove_from_config`. This keeps every existing `remove_caddy_route` caller unchanged.

- [ ] **Step 5: Thread `path` through the callers.**
  - `caddy_route`, `caddy_application_routes`, the restore loop and `push_caddy_route` pass `domain.path.as_deref()` (or `route.path`).
  - Each skips `certificates::apply` when the path is `Some`.
  - The restore loop calls `route_present(config, id, host, route.https && route.path.is_none())`.
  - Update the 8 existing test calls in `operations/tests.rs` to pass `None`.

- [ ] **Step 6: Run the tests and confirm they pass.**
Run: `cd apps/api && cargo test --lib deployment && cargo clippy --all-targets -- -D warnings`
Expected: PASS, with no new warnings.

### Task 3: Path field in the domain dialog

**Files:**
- Modify: `apps/ui/src/lib/clients/project-client.ts` (`DomainInput.path?: string | null`, `DomainDto.path: string | null`)
- Modify: `apps/ui/src/lib/components/projects/domain-form-dialog.svelte`
- Modify: `apps/ui/src/lib/components/projects/application-domains-tab.svelte` (show `host + path` in the list, and pass `existingHosts` only for root domains: `domains.filter((d) => !d.path).map((d) => d.host)`)
- Modify: `apps/ui/src/lib/i18n/messages/{en,es}/projects.toml` (`[domain_form]`: `path`, `path_placeholder`, `path_hint`, `path_https_inherited`)
- Test: `apps/ui/src/lib/components/projects/domain-form-dialog.test.ts`

**Interfaces:**
- Consumes: the API fields `path` (request and DTO) from Task 1.

- [ ] **Step 1: Write the failing tests:**

```ts
it('sends the path and hides HTTPS and the www twin for a path domain', async () => {
	const onSubmit = vi.fn().mockResolvedValue(undefined);
	render(DomainFormDialog, { open: true, onSubmit });
	await fireEvent.input(screen.getByRole('textbox', { name: 'Host' }), { target: { value: 'example.com' } });
	await fireEvent.input(screen.getByRole('textbox', { name: 'Path' }), { target: { value: '/api' } });
	await fireEvent.input(screen.getByLabelText('Internal port (Caddy target)'), { target: { value: '8080' } });
	expect(screen.queryByLabelText('HTTPS')).toBeNull();
	expect(screen.queryByLabelText('Also add www.example.com')).toBeNull();
	await fireEvent.submit(screen.getByRole('textbox', { name: 'Host' }).closest('form')!);
	await waitFor(() => expect(onSubmit).toHaveBeenCalledOnce());
	expect(onSubmit.mock.calls[0][0]).toMatchObject({ host: 'example.com', path: '/api', port: 8080 });
});

it('hides the path field for local hosts', async () => {
	render(DomainFormDialog, { open: true, onSubmit: vi.fn() });
	await fireEvent.input(screen.getByRole('textbox', { name: 'Host' }), { target: { value: 'localhost' } });
	expect(screen.queryByRole('textbox', { name: 'Path' })).toBeNull();
});
```

- [ ] **Step 2: Run them and confirm they fail.**
Run: `cd apps/ui && bunx vitest run src/lib/components/projects/domain-form-dialog.test.ts`
Expected: FAIL. There is no `Path` textbox yet.

- [ ] **Step 3: Implement the dialog changes.**
  - Add `let path = $state('')`, reset from `domain?.path ?? ''` on open.
  - Put a monospace `Input` labelled `path` under the host, shown when `!isLocal`. Hint copy:
    - en: `"Optional. /api sends example.com/api/* here and removes /api before it reaches the app."`
    - es: `"Opcional. /api envía example.com/api/* aquí y quita /api antes de llegar a la app."`
  - Submit sends `path: path.trim()`. An empty string means "no path" on create and "clear the path" on edit, and the server normalizes it.
  - When `path.trim()` is set, hide the HTTPS switch and `CertificateFields` and show `path_https_inherited`:
    - en: `"Uses the HTTPS settings of {host}."`
    - es: `"Usa la configuración HTTPS de {host}."`
  - Also gate `wwwVariant` on an empty path.

- [ ] **Step 4: Run the tests and check types.**
Run: `cd apps/ui && bunx vitest run src/lib/components/projects/domain-form-dialog.test.ts src/lib/components/projects/application-domains-tab.test.ts && bun run check`
Expected: PASS, and `svelte-check` reports 0 errors.

---

Out of scope for v1, and why:
- Public-edge and private-relay servers: their Caddyfile manifest is one site block per host. Supporting paths there means changing the root-owned remote helper.
- A "keep prefix" toggle.
- Exposing `path` to the assistant and MCP tools: they pass `path: None`.
- Path-aware maintenance `errors` routes.
