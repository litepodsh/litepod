# Security fixes — noviembre 2026

Lista de la auditoría de seguridad de Litepod. Prioridad: **Cloud** primero, **self-hosted** después.
Cada ítem lleva archivo:línea y la corrección. Marcá los checkboxes a medida que avances.

> Nota: los hallazgos se verificaron por lectura de código (y el #1 se reprodujo con las mismas
> versiones de `git2`/`tar`). No reemplaza un pentest: probá cada fix en staging antes de confiar en él.

---

## Cloud — Críticos

### 1. Repo malicioso roba archivos del control plane — ✅ HECHO
- Causa: el repo se clona en el control plane y se empaqueta con `tar::Builder`, que sigue symlinks por defecto.
- Archivo: `apps/api/src/servers/runtime.rs:784`
- [x] `builder.follow_symlinks(false)` antes de `append_dir_all`
- [x] Confirmar que no haya otros `append_dir_all` en la ruta de deploy (solo este; el de railpack es test)
- [ ] `cargo build` completo en CI
- [ ] Auditar si algún cliente pudo usarlo; si no se descarta, **rotar `ENCRYPTION_KEY` y claves SSH**

### 2. XSS en el origen del panel vía dominio de mantenimiento — ✅ HECHO
- Causa: un inquilino podía registrar el host del panel como dominio de su app y servir su JS desde ese origen.
- Archivos: `apps/api/src/deployment/domains/validate.rs`, `crud.rs:409,503`, `tests.rs`
- [x] Guard `reject_reserved_host` que rechaza el host del control plane (server-domain + `PUBLIC_URL`)
- [x] Llamarlo al crear y al actualizar dominio
- [x] Test que fija el rechazo
- [ ] Auditar DB: `SELECT application_id, host FROM application_domains WHERE lower(host) = '<host-del-panel>';`
- [ ] (Opcional) Rechazar al guardar server-domain un host que ya use alguna app

---

## Cloud — Altos

### 3. Submódulos Git sin validar (SSRF + fuga de token) — ✅ HECHO
- Reusan las credenciales del repo padre y no validan la URL del submódulo.
- Archivo: `apps/api/src/deployment/git.rs:88`
- [x] Validar cada URL de submódulo (SSRF guard) — `vet_submodule_url`, resolver bloqueante + `ssrf::is_public`
- [x] Enviar credenciales solo si el host coincide con el del repo padre (y `allow_private` solo para proveedor self-hosted)
- [x] Tests de `vet_submodule_url` (host match, host ajeno, relativo, scp, IP interna)

### 4. Reset / cambio de contraseña no revoca PAT ni tokens OAuth — ✅ HECHO
- Un intruso conserva acceso vía token aunque se cambie la clave.
- Archivos: `apps/api/src/users/password_reset.rs:160`, `cqrs/commands/account.rs:48`
- [x] Revocar personal access tokens del usuario (en reset y en cambio de contraseña)
- [x] Revocar grants OAuth del usuario (en ambos)
- [x] Test de que tras el cambio ningún token viejo sirve
  - Nota: el test va sobre `change_password` (solo DB). El reset comparte las mismas llamadas de revocación, pero su test end-to-end necesita el store efímero (Dragonfly), deshabilitado en tests unitarios.

---

## Cloud — Medios

### 5. DNS rebinding + rangos IPv6 faltantes — ✅ HECHO (con residuales)
- Se valida la IP y luego el cliente vuelve a resolver; no se bloquean NAT64 (`64:ff9b::/96`) ni 6to4 (`2002::/16`).
- Archivo: `apps/api/src/security/ssrf.rs:127,178`
- [x] Añadir NAT64 y 6to4 a `is_public` (extrae la IPv4 embebida y la valida; bloquea `64:ff9b::a9fe:a9fe` y `2002:a9fe:a9fe::`)
- [x] `resolve_validated`: resuelve una vez y devuelve las IPs para fijar la conexión (anti-rebinding)
- [x] Fijar la IP validada en el fetch del asistente (`assistant/web.rs`, cliente por salto)
- [x] Fijar la IP validada en el envío de notificaciones (`notifications/dispatch.rs`)
- [x] Tests de NAT64/6to4 (privada rechazada, pública aceptada)
- [ ] **Residual:** backup S3 (`backup/target.rs`) usa aws-sdk, otro stack HTTP → aún re-resuelve; pinear requiere un connector propio
- [ ] **Residual (cliente de proveedor):** `git/git_providers/provider.rs` usa reqwest y aún re-resuelve; arreglable con `resolve_to_addrs` como aquí (pendiente)
- [~] **Riesgo conocido aceptado (git2):** el clone de `deployment/git.rs` re-resuelve y git2 no permite fijar IP fácilmente. Ventana menor (host de proveedor fijo); el hueco real es el clone anónimo con URL a mano. No se corrige por ahora.

### 6. Apropiación de dominio entre inquilinos — [~] RIESGO CONOCIDO ACEPTADO
- `(host, port)` es único global y no se exige probar propiedad del dominio (`crud.rs:374` no verifica; `validate_dns` es solo informativo).
- Archivo: `apps/api/migrations/0001_init.sql:314`
- Severidad según topología:
  - **Cloud (public-edge, IP+Caddy por servidor):** sobre todo *squatting/DoS* (bloquear al dueño legítimo). Hijack directo NO, salvo DNS colgante que ya apunte a ese server.
  - **Ingress central / Caddy compartido (self-hosted):** sí hay takeover + emisión de cert si el host ya apunta a la IP compartida. Más grave.
- Decisión (2026-11): **no se corrige por ahora.** El fix de libro (verificación TXT antes de rutear/emitir cert) es una feature con migración, endpoint, UI y fricción nueva para cada dominio custom.
- [ ] Si se retoma: verificación de propiedad (TXT) antes de activar un dominio público, al menos en ingress central/compartido

### 7. Faltan CSP de scripts y HSTS en el panel — ✅ HECHO (parcial)
- Cualquier XSS tiene más impacto sin estas cabeceras.
- Archivo: `apps/api/src/router/router.rs` (`ui_fallback`)
- [x] `Strict-Transport-Security` (`max-age=31536000; includeSubDomains`), solo si `X-Forwarded-Proto: https` (no bloquea self-hosted en http)
- [x] CSP endurecida sin tocar scripts: `base-uri 'self'; object-src 'none'` (+ `frame-ancestors 'none'` que ya estaba)
- [x] Tests: valor de CSP y HSTS presente/ausente según esquema
- [ ] **Pendiente (`script-src` estricto):** el shell de SvelteKit tiene un `<script>` inline de hidratación sin nonce cuyo hash cambia por build. Requiere CSP con hashes en el build de la UI (`kit.csp` modo hash) + probar en browser Turnstile (challenges.cloudflare.com) y Monaco (workers/blob). No se hace a ciegas desde el server.

### 8. Rate limit de login solo por IP — ✅ HECHO (opción 3: cuenta + IP tolerante)
- Archivos: `apps/api/src/security/middleware/rate_limit.rs`, `apps/api/src/handler/auth.rs`
- [x] Ban por **cuenta** (email digerido) como control principal de fuerza bruta: bloquea la cuenta atacada en cualquier IP, chequeado antes de verificar la contraseña; se limpia al loguear bien (incl. TOTP)
- [x] Ban de **IP** vuelto backstop: umbral ×10 (`IP_BAN_ATTEMPT_MULTIPLIER`) para que una IP compartida/NAT no se bloquee con fallos cotidianos
- [x] Fail-open si Dragonfly no está (siguen el rate-limiter y el ban de IP)
- [x] Test del keying por cuenta (normaliza casing/espacios, no expone el email)
- Tradeoff aceptado: un ban por cuenta habilita un DoS de bloqueo dirigido a una víctima; acotado por la ventana (`auth_ban_minutes`) y el captcha ya existente.
- Nota: el ban por cuenta end-to-end necesita Dragonfly (deshabilitado en tests unitarios), igual que #4; el test cubre la lógica de clave.

### 9. Phishing por OAuth — ✅ HECHO (parcial, UI)
- Registro abierto; el nombre del cliente lo elige quien registra la app y se mostraba como si fuera de confianza.
- Archivos: `apps/ui/src/routes/oauth/consent/+page.svelte`, `apps/ui/src/lib/i18n/messages/{en,es}/organizations.toml`
- [x] Advertencia visible siempre: el nombre lo eligió la app y litepod no lo verificó; aprobar solo si la reconoce e inició el flujo
- [x] Ya existían: destino (`redirect_host`) y alerta `remote` para el caso hosted/https (el peligroso)
- [x] Verificado: paridad i18n EN/ES y `svelte-check` 0 errores
- [ ] **Opcional (más fuerte):** registro de clientes con aprobación previa, o lista de clientes conocidos, en vez de registro abierto. No se hizo (cambio de producto).

### 10. Una sola `ENCRYPTION_KEY` protege las claves SSH de todos los clientes
- Radio de impacto enorme si se filtra.
- Archivo: `apps/api/src/utils/security.rs:21`
- [ ] Clave por cliente o KMS

### 11. Se acepta IP no pública como host SSH
- `127.0.0.1`, `169.254.169.254`, etc. permiten sondear la red interna.
- Archivo: `apps/api/src/servers/model.rs:179`
- [ ] Bloquear IPs no públicas como host SSH en cloud

---

## Self-hosted — Críticos / Altos

### 12. Admin de Caddy sin auth en la red compartida
- `0.0.0.0:2019` alcanzable por cualquier app con dominio; puede reescribir todas las rutas.
- Archivos: `apps/Caddyfile/Caddyfile.prod:2`, `apps/api/src/deployment/compose_standalone/overrides.rs:47`
- [ ] Red interna solo para el control plane; sacar el admin de Caddy de la red de apps

### 13. BuildKit privilegiado expuesto en la red — ✅ HECHO
- `tcp://0.0.0.0:1234`, privilegiado; no se encontró código que lo use.
- Archivo: `apps/podman-compose-prod.yml:6`, `apps/podman-compose-staging.yml:8`
- [x] Confirmar que no se usa y eliminar el servicio (o aislarlo/autenticarlo)
  - Confirmado: el build real va por `scripts/railpack-build.sh` → `buildkitd --rootless` en unix socket (`unix:///tmp/buildkit-runtime/buildkitd.sock`), en un contenedor aislado no-root (`Dockerfile.railpack-builder`). Ninguna conexión a `litepod-buildkit:1234`.
  - Eliminado servicio `buildkit` y volumen `buildkit_cache` de `podman-compose-prod.yml` y `podman-compose-staging.yml`.
  - `buildkit` se mantiene en la lista de nombres reservados (`compose/validation.rs:13`, `compose-reserved-services.ts:26`).

### 14. Compose sin restringir `privileged` / montajes de host / socket Podman — ✅ HECHO
- El server `local` se comparte entre organizaciones; un Editor podría tomar el host.
- Archivo: `apps/api/src/deployment/compose/validation.rs`
- [x] Rechazar `privileged`, bind mounts de host y el socket de Podman para no-sysadmin
  - `validation.rs`: nueva `privileged_services_in_compose` + `privileged_services_message` (+ tests).
  - `store.rs::write_content` (chokepoint único): rechazo incondicional de `unnamed_mounts_in_compose` (host paths, binds, anónimos — cubre el socket de Podman).
  - `handler/compose.rs`: rechazo de mounts en save y validate; rechazo de `privileged` solo si `!app.user.is_sysadmin` (Editor/Admin bloqueado, SysAdmin permitido).
  - `assistant/actions/propose.rs`: `require_no_privileged` en `propose_compose_file` y `checked_compose` (el asistente es Editor, nunca SysAdmin).

### 15. WebSockets de terminal sin validar `Origin` — ✅ HECHO
- Una app en subdominio hermano del panel podría abrir terminal como el admin que la visita.
- Archivos: `apps/api/src/podman/terminal/exec.rs`, `host.rs`
- [x] Validar `Origin` en el upgrade de WebSocket
  - `security/middleware/csrf.rs`: `websocket_origin_allowed` + `allowed_origins` (CORS_ORIGINS + server domain).
  - Rechazo 403 en los 3 upgrades: `host.rs` (consola del host), `exec.rs::server_exec_ws_handler` y `exec.rs::exec_ws_handler`.
  - `Origin` ausente = cliente no-browser (curl/CLI), permitido (no puede robar cookie).

---

## Self-hosted — Medios

### 16. Actualizaciones sin verificar firma ni digest — [~] HECHO parcial (digest)
- Archivo: `apps/api/src/updater/`
- [x] Fijar imagen por digest (pin inmutable, fallback a tag)
  - `check.rs`: `ChannelEntry.digest` + `app_digest()` + `is_valid_digest` (rechaza digest inválido, no cae al tag).
  - `update.rs`: `UpdateState.latest_digest`; `detect_once` lo resuelve; `apply_now` pull `repo@sha256:…` cuando hay digest.
  - `handler/system.rs`: pasa `snapshot.latest_digest` al apply (sin round-trip por UI).
  - Release pipeline: `merge-version-manifest.py` acepta `DIGEST` (valida `sha256:` + 64 hex) y lo publica.
  - Detección de versión: con ref digest no hay tag → `running_version` cae a `LITEPOD_VERSION` (baked en la imagen), correcto.
- [ ] **Pendiente (cosign):** verificar firma sigstore de la imagen. Requiere firmar en CI + clave pública/rekor embebida en el binario. No se hace en esta pasada.
- [ ] **Pendiente (firma del manifest):** version.json sigue sin firmar (solo HTTPS). Mismo esquema de clave, o minisign.

### 17. `/auth/setup` abierto hasta crear el primer admin — ✅ HECHO
- Archivos: `apps/api/src/handler/auth.rs`, `apps/api/src/utils/cookie.rs`
- [x] Token de instalación de un solo uso para el setup
  - `dtos/auth.rs`: `SetupRequest.setup_token` (`#[serde(default)]`).
  - `handler/auth.rs`: `setup_token_matches` (comparación constant-time) + rechazo 403 (`AppError::SetupTokenInvalid`) cuando `SETUP_TOKEN` está configurado y no coincide.
  - `scripts/install.sh`: genera `SETUP_TOKEN` (openssl rand -hex 32), lo escribe en `.env` y lo imprime al operador.
  - Compose prod/staging: `SETUP_TOKEN: ${SETUP_TOKEN:-}`.
  - UI: campo "Setup token" en el flujo de primer admin + i18n EN/ES.
  - Nota: cookie `Secure` ya estaba corregida (`cookie.rs::secure` sigue `public_base_url` https); el item restante era solo el token.
  - Retrocompat: `SETUP_TOKEN` sin configurar (dev, installs legacy) → setup abierto como antes.

### 18. `X-Forwarded-For` falsificable desde la red interna — ✅ HECHO
- Una app interna evita el ban de login falsificando la IP del cliente.
- Archivo: `apps/api/src/security/middleware/rate_limit.rs`
- [x] Confiar solo en la IP real de Caddy
  - `peer_may_set_forwarded_for`: ya no confía en rangos privados (`is_private` / IPv6 unique-local).
  - Confía solo en loopback + link-local + la IP resuelta de `litepod-caddy` (`proxy_peer`, TTL 60s para re-resolver tras recreate de Caddy).
  - `TRUSTED_PROXY_IPS` sigue como override explícito (LB en IP pública, etc.).
  - Tests: peer privado ya no es creído sin el proxy; `trusted_proxy_peer` cubre loopback/link-local/proxy/private/public.

---

## Dependencias

### 19. Advisories de dependencias
- [ ] Rust: `cargo audit` reporta 28 vulnerabilidades — triar cuáles afectan a producción
- [ ] UI: `bun audit` reporta 54 (3 críticas, 10 altas) — empezar por **DOMPurify** (el panel lo usa para sanear HTML)

---

## Orden recomendado
1. **Hoy:** #1 y #2 (hechos → cerrar los sub-ítems de auditoría/CI).
2. **Esta semana:** #3, #4 y, en self-hosted, #12 y #13.
3. **Después:** #5–#11, #14–#18 y el triaje de #19.
