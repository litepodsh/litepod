# SSO con Keycloak (OIDC)

[← SSO: lo común](sso.md) · Otros IdP: [Entra ID](oidc.md) · [Authentik](authentik.md) · [Logto](logto.md)

Protocolo: **OIDC**. Keycloak también habla SAML ([saml.md](saml.md)), pero con OIDC basta y es
más simple. "SSO" es el objetivo; OIDC es el protocolo con el que se consigue.

## Qué se intercambia

| Campo en Litepod | Dónde está en Keycloak |
|---|---|
| **URL del issuer** | `https://<host-keycloak>/realms/<realm>` (sin `/` final) |
| **Client ID** | **Clients → litepod → Settings → Client ID** |
| **Client secret** | **Clients → litepod → Credentials → Client secret** |
| **Claim de grupos** | `groups` (mapper *Group Membership*, paso 2.5) |
| **URI de redirección** *(de Litepod a Keycloak)* | **Clients → litepod → Settings → Valid redirect URIs** |

Keycloak manda los grupos **por nombre** (`litepod-devs`), no por ID como Entra. En el mapeo de
grupos de Litepod se pone ese nombre.

## 1. Levantar Keycloak (self-hosted)

Sáltate esto si ya tienes Keycloak.

### 1.1 Prueba rápida (modo dev, sin TLS)

```bash
podman run -d --name keycloak -p 8081:8080 \
  -e KC_BOOTSTRAP_ADMIN_USERNAME=admin \
  -e KC_BOOTSTRAP_ADMIN_PASSWORD=admin \
  quay.io/keycloak/keycloak:26.0 start-dev
```

Consola: http://localhost:8081 (`admin` / `admin`). Datos en memoria del contenedor: se pierden
al borrarlo. Litepod solo acepta un issuer `http://` si la API corre en modo dev; en producción
exige `https://`.

### 1.2 Producción (detrás de Caddy, con Postgres)

```yaml
# keycloak-compose.yml
services:
  keycloak-db:
    image: docker.io/library/postgres:17
    environment:
      POSTGRES_DB: keycloak
      POSTGRES_USER: keycloak
      POSTGRES_PASSWORD: <cambia-esto>
    volumes: [keycloak-db:/var/lib/postgresql/data]

  keycloak:
    image: quay.io/keycloak/keycloak:26.0
    command: start
    environment:
      KC_DB: postgres
      KC_DB_URL: jdbc:postgresql://keycloak-db:5432/keycloak
      KC_DB_USERNAME: keycloak
      KC_DB_PASSWORD: <cambia-esto>
      KC_HOSTNAME: https://sso.tuempresa.com
      KC_HTTP_ENABLED: "true"          # TLS lo termina Caddy
      KC_PROXY_HEADERS: xforwarded
      KC_BOOTSTRAP_ADMIN_USERNAME: admin
      KC_BOOTSTRAP_ADMIN_PASSWORD: <cambia-esto>
    ports: ["8081:8080"]
    depends_on: [keycloak-db]

volumes:
  keycloak-db:
```

```caddyfile
sso.tuempresa.com {
    reverse_proxy localhost:8081
}
```

```bash
podman compose -f keycloak-compose.yml up -d
```

Tras el primer login cambia la contraseña de `admin` (o crea otro admin y borra el de arranque).

**Importante**: la API de Litepod tiene que poder llegar a `https://sso.tuempresa.com` (lee
`/.well-known/openid-configuration` y las claves). Si corren en la misma máquina con contenedores,
comprueba que el DNS resuelve desde dentro del contenedor de la API.

## 2. En Keycloak: configurar el realm y el client

### 2.1 Crear el realm

1. Arriba a la izquierda, selector de realm → **Create realm**.
2. **Realm name**: `tuempresa` → **Create**.

No uses el realm `master`: es para administrar Keycloak.

### 2.2 Crear usuarios

1. **Users → Add user**.
2. **Username** `test`, **Email** `test@tuempresa.com`, activa **Email verified** → **Create**.
3. Pestaña **Credentials → Set password**, desactiva **Temporary** → **Save**.

El correo **tiene que** ser del dominio verificado en Litepod ([sso.md → 3](sso.md#3-verificar-el-dominio-de-correo)).
Sin email, Litepod rechaza el login ("the ID token carries no email address").

Si tus usuarios vienen de un AD/LDAP: **User federation → Add LDAP providers**. Keycloak los
expone por OIDC igual.

### 2.3 Crear grupos

1. **Groups → Create group** → `litepod-admins`. Repite con `litepod-devs`.
2. **Users → test → Groups → Join group** → `litepod-devs`.

### 2.4 Crear el client

1. **Clients → Create client**.
2. **General settings**: **Client type** `OpenID Connect`, **Client ID** `litepod` → **Next**.
3. **Capability config**:
   - **Client authentication**: **On** (client confidencial, con secreto).
   - **Authentication flow**: solo **Standard flow**. Desmarca *Direct access grants*.
   → **Next**.
4. **Login settings**:
   - **Valid redirect URIs**: `<base>/api/v1/auth/sso/callback`
     (ej. `https://litepod.tuempresa.com/api/v1/auth/sso/callback`).
   - **Web origins**: `<base>`.
   → **Save**.
5. Pestaña **Credentials** → copia el **Client secret**.

Opcional, recomendado: pestaña **Advanced → Advanced settings → Proof Key for Code Exchange Code
Challenge Method** = `S256`. Litepod ya manda PKCE; esto obliga a Keycloak a exigirlo.

### 2.5 Mandar los grupos en el token

1. **Clients → litepod → Client scopes** → `litepod-dedicated`.
2. **Configure a new mapper** (o **Add mapper → By configuration**) → **Group Membership**.
3. Rellena:
   - **Name**: `groups`
   - **Token Claim Name**: `groups`
   - **Full group path**: **Off** (si no, llega `/litepod-devs` con barra)
   - **Add to ID token**: **On**
4. **Save**.

Litepod pide los scopes `openid email profile`. El scope dedicado del client se incluye siempre,
así que no hace falta pedir un scope `groups`.

### 2.6 Comprobar

**Clients → litepod → Client scopes → Evaluate** → elige el usuario `test` → **Generated ID
token**. Debe traer `email: test@tuempresa.com` y `groups: ["litepod-devs"]`.

## 3. En Litepod: crear la conexión

Antes: dominio verificado en Litepod ([sso.md → 3.1](sso.md#31-en-litepod)).

1. **Organización** → pestaña **Autenticación** → **Proveedores de identidad** →
   **Agregar proveedor de identidad**.
2. Rellena:

   | Campo | Valor |
   |---|---|
   | **Nombre** | `Keycloak` |
   | **Protocolo** | **OpenID Connect** |
   | **URL del issuer** | `https://sso.tuempresa.com/realms/tuempresa` |
   | **Client ID** | `litepod` |
   | **Client secret** | El de **Credentials** |
   | **Claim de grupos** | `groups` |
   | **Rol base para miembros nuevos** | El mínimo |
   | **Crear cuentas en el primer inicio de sesión (just-in-time)** | Marcado |
   | **Activado** | Marcado |

3. **Guardar**.
4. **Editar** la conexión y copia la **URI de redirección**. Debe ser idéntica a la de
   **Valid redirect URIs** en Keycloak.

### 3.1 Grupos → roles

1. **Organización → Acceso**: crea los grupos `admins` y `devs` y asígnales roles.
2. **Autenticación → Mapeo de grupos → Agregar mapeo**:
   - ID del grupo en el IdP: `litepod-admins` → grupo `admins`.
   - ID del grupo en el IdP: `litepod-devs` → grupo `devs`.
3. **Guardar**.

Detalle en [sso.md → 5](sso.md#5-grupos-y-roles). Aquí el "ID" es el **nombre** del grupo de Keycloak.

## 4. Probar

1. Ventana privada → `<base>/login` → **Continuar con SSO** → `test@tuempresa.com`.
2. Te lleva al login de Keycloak. Entra.
3. Vuelves a Litepod con sesión, dentro de la org y en el grupo `devs`.

## 5. Si falla

Keycloak registra cada intento en **Events** (actívalo en **Realm settings → Events → User
events settings → Save events**).

| Síntoma | Causa | Arreglo |
|---|---|---|
| Keycloak: `Invalid parameter: redirect_uri` | La URI no coincide | Copia la de Litepod en **Valid redirect URIs** |
| Keycloak: `Client not found` | **Client ID** mal escrito, o realm equivocado en el issuer | Revisa issuer y Client ID |
| Litepod: `could not read the identity provider's configuration` | Issuer mal, con `/` final, o la API no llega a Keycloak | Abre `<issuer>/.well-known/openid-configuration` desde la máquina de la API |
| Litepod: `Issuer must use https` | Issuer `http://` fuera de modo dev | Pon Keycloak detrás de TLS (1.2) |
| Litepod: `the code exchange was refused` | Client secret incorrecto o regenerado | Pega el actual en **Editar → Client secret** |
| Litepod: `the ID token did not verify` | `KC_HOSTNAME` distinto del host del issuer | El issuer de Litepod debe ser exactamente el `issuer` del `.well-known` |
| Litepod: `the ID token carries no email address` | Usuario sin email | Pon **Email** al usuario |
| Entra pero sin grupo | Falta el mapper, o *Full group path* en On | 2.5 |

## Checklist

- [ ] Realm propio (no `master`), usuarios con email del dominio verificado.
- [ ] Client `litepod` confidencial, solo Standard flow, redirect URI exacta.
- [ ] Mapper *Group Membership* `groups`, Full group path Off, en ID token.
- [ ] Conexión en Litepod **Activado** con issuer `https://<host>/realms/<realm>`.
- [ ] Mapeo de grupos por **nombre**.
- [ ] Login de prueba OK.
