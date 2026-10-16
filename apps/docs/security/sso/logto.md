# SSO con Logto (OIDC)

[← SSO: lo común](sso.md) · Otros IdP: [Entra ID](oidc.md) · [Keycloak](keycloak.md) · [Authentik](authentik.md)

Protocolo: **OIDC**. Logto no actúa como IdP SAML para apps propias de forma práctica: usa OIDC.
Sirve igual con Logto Cloud o self-hosted; solo cambia la URL.

## Limitación: sin grupos

Logto no tiene "grupos": tiene **roles** y **organizaciones**, y solo los mete en el ID token si
la app pide los scopes `roles` o `urn:logto:scope:organizations`. Litepod pide solo
`openid email profile`, así que **con Logto no llega claim de grupos** y el
[mapeo de grupos](sso.md#5-grupos-y-roles) no hace nada.

Qué funciona: login, JIT y el **Rol base**. Los grupos de Litepod se gestionan a mano en
**Organización → Acceso**. Si necesitas grupos desde el IdP, usa [Keycloak](keycloak.md) o
[Authentik](authentik.md).

## Qué se intercambia

| Campo en Litepod | Dónde está en Logto |
|---|---|
| **URL del issuer** | **Applications → Litepod → Endpoints & Credentials → Issuer endpoint**: `https://<host-logto>/oidc` |
| **Client ID** | Misma sección → **App ID** |
| **Client secret** | Misma sección → **App secret** |
| **Claim de grupos** | Déjalo en `groups` (no llegará; el campo es obligatorio) |
| **URI de redirección** *(de Litepod a Logto)* | **Applications → Litepod → Redirect URIs** |

## 1. Levantar Logto (self-hosted)

Sáltate esto si usas Logto Cloud o ya lo tienes.

```yaml
# logto-compose.yml
services:
  logto-db:
    image: docker.io/library/postgres:17-alpine
    environment:
      POSTGRES_USER: postgres
      POSTGRES_PASSWORD: <cambia-esto>
    volumes: [logto-db:/var/lib/postgresql/data]

  logto:
    image: docker.io/svhd/logto:latest
    entrypoint: ["sh", "-c", "npm run cli db seed -- --swe && npm start"]
    environment:
      TRUST_PROXY_HEADER: 1
      DB_URL: postgres://postgres:<cambia-esto>@logto-db:5432/logto
      ENDPOINT: https://auth.tuempresa.com          # OIDC, lo que ve Litepod
      ADMIN_ENDPOINT: https://logto-admin.tuempresa.com  # consola
    ports: ["3001:3001", "3002:3002"]
    depends_on: [logto-db]

volumes:
  logto-db:
```

```caddyfile
auth.tuempresa.com {
    reverse_proxy localhost:3001
}
logto-admin.tuempresa.com {
    reverse_proxy localhost:3002
}
```

```bash
podman compose -f logto-compose.yml up -d
```

Abre `https://logto-admin.tuempresa.com`: la primera visita crea la cuenta de administrador.
Cambia `latest` por una versión fija cuando lo pases a producción.

**Importante**: la API de Litepod tiene que poder llegar a `https://auth.tuempresa.com`. Litepod
exige `https://` en el issuer salvo en modo dev.

## 2. En Logto: configurar

### 2.1 Login con email y contraseña

1. **Sign-in experience → Sign-up and sign-in**.
2. **Sign-in identifiers**: **Email address**, con **Password**.
3. **Save changes**.

### 2.2 Usuarios

1. **User management → Add user**.
2. **Email** `test@tuempresa.com` (y username si lo pide) → **Add user**.
3. Copia la contraseña generada.

El email **tiene que** ser del dominio verificado en Litepod. Sin email, Litepod rechaza el login
("the ID token carries no email address").

### 2.3 Crear la aplicación

1. **Applications → Create application**.
2. Elige **Traditional web** (sin framework, o cualquier framework del bloque *Traditional web*).
   No elijas *Single page app* ni *Native*: esos no tienen secreto.
3. **Application name**: `Litepod` → **Create application**.
4. En la app, pestaña **Settings**:
   - **Redirect URIs**: `<base>/api/v1/auth/sso/callback`
     (ej. `https://litepod.tuempresa.com/api/v1/auth/sso/callback`).
   - **Post sign-out redirect URIs**: `<base>/login` (opcional).
   - **Save changes**.
5. Sección **Endpoints & Credentials**: copia **Issuer endpoint**, **App ID** y **App secret**.

## 3. En Litepod: crear la conexión

Antes: dominio verificado en Litepod ([sso.md → 3.1](sso.md#31-en-litepod)).

1. **Organización** → **Autenticación** → **Proveedores de identidad** → **Agregar proveedor de
   identidad**.
2. Rellena:

   | Campo | Valor |
   |---|---|
   | **Nombre** | `Logto` |
   | **Protocolo** | **OpenID Connect** |
   | **URL del issuer** | `https://auth.tuempresa.com/oidc` (sin `/` final) |
   | **Client ID** | **App ID** |
   | **Client secret** | **App secret** |
   | **Claim de grupos** | `groups` |
   | **Rol base para miembros nuevos** | El rol que quieres para todos (no habrá grupos del IdP) |
   | **Crear cuentas en el primer inicio de sesión (just-in-time)** | Marcado |
   | **Activado** | Marcado |

3. **Guardar**.
4. **Editar** → copia la **URI de redirección** y compárala con la de Logto: idénticas.

## 4. Probar

1. Ventana privada → `<base>/login` → **Continuar con SSO** → `test@tuempresa.com`.
2. Login de Logto → entra.
3. Vuelves a Litepod con sesión, dentro de la org y con el **Rol base**.
4. Asigna grupos al usuario a mano en **Organización → Acceso**.

## 5. Si falla

Logto registra cada intento en **Audit logs**.

| Síntoma | Causa | Arreglo |
|---|---|---|
| Logto: `redirect_uri did not match any of the client's registered redirect_uris` | URI no coincide | Copia la de Litepod en **Redirect URIs** y **Save changes** |
| Litepod: `could not read the identity provider's configuration` | Issuer sin `/oidc`, o la API no llega a Logto | Abre `<issuer>/.well-known/openid-configuration` desde la máquina de la API |
| Litepod: `the ID token did not verify` | `ENDPOINT` de Logto distinto de la URL del issuer | El issuer de Litepod debe ser exactamente el `issuer` del `.well-known` |
| Litepod: `the code exchange was refused` | App secret incorrecto, o app tipo SPA/Native | App **Traditional web** + secreto actual |
| Litepod: `the ID token carries no email address` | Usuario sin email | 2.2 |
| Entra pero sin grupos | Esperado con Logto | Ver *Limitación* arriba |

## Checklist

- [ ] Logto con HTTPS y alcanzable desde la API de Litepod.
- [ ] Sign-in por email + contraseña activo.
- [ ] Usuarios con email del dominio verificado.
- [ ] App **Traditional web** con redirect URI exacta.
- [ ] Conexión en Litepod con issuer `https://<host>/oidc`.
- [ ] Login de prueba OK; grupos asignados a mano.
