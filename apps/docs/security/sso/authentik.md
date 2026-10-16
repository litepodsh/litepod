# SSO con Authentik (OIDC)

[← SSO: lo común](sso.md) · Otros IdP: [Entra ID](oidc.md) · [Keycloak](keycloak.md) · [Logto](logto.md)

Protocolo: **OIDC** (en Authentik se llama *OAuth2/OpenID Provider*). Authentik también tiene un
*SAML Provider* ([saml.md](saml.md)), pero OIDC es lo recomendado.

## Qué se intercambia

| Campo en Litepod | Dónde está en Authentik |
|---|---|
| **URL del issuer** | **Providers → litepod → OpenID Configuration Issuer**: `https://<host>/application/o/<slug>/` (**con** `/` final) |
| **Client ID** | **Providers → litepod → Edit → Client ID** |
| **Client secret** | **Providers → litepod → Edit → Client Secret** |
| **Claim de grupos** | `groups` (viene por defecto en el scope `profile`) |
| **URI de redirección** *(de Litepod a Authentik)* | **Providers → litepod → Edit → Redirect URIs** |

Ojo con el issuer: Authentik lo publica **con barra final**. Si en Litepod lo pones sin ella, la
validación del token falla. Cópialo tal cual de la pantalla del provider.

Authentik manda los grupos **por nombre**. En el mapeo de Litepod se pone ese nombre.

## 1. Levantar Authentik (self-hosted)

Sáltate esto si ya tienes Authentik.

```bash
mkdir authentik && cd authentik
curl -O https://goauthentik.io/docker-compose.yml
echo "PG_PASS=$(openssl rand -base64 36 | tr -d '\n')" >> .env
echo "AUTHENTIK_SECRET_KEY=$(openssl rand -base64 60 | tr -d '\n')" >> .env
podman compose pull
podman compose up -d
```

El compose oficial trae Postgres y expone `9000` (HTTP) y `9443` (HTTPS). Ponlo detrás de Caddy:

```caddyfile
auth.tuempresa.com {
    reverse_proxy localhost:9000
}
```

Configuración inicial: abre `https://auth.tuempresa.com/if/flow/initial-setup/` (con la barra
final) y crea la contraseña de `akadmin`. Luego **Admin interface** arriba a la derecha.

**Importante**: la API de Litepod tiene que poder llegar a `https://auth.tuempresa.com`. Litepod
exige `https://` en el issuer salvo en modo dev.

## 2. En Authentik: configurar

### 2.1 Usuarios

1. **Directory → Users → Create**.
2. **Username** `test`, **Name** `Test`, **Email** `test@tuempresa.com` → **Create**.
3. En el usuario → **Set password**.

El email **tiene que** ser del dominio verificado en Litepod. Sin email, Litepod rechaza el login.

### 2.2 Grupos

1. **Directory → Groups → Create** → `litepod-admins`. Repite con `litepod-devs`.
2. Abre `litepod-devs` → **Users → Add existing user** → `test`.

### 2.3 Aplicación + provider

1. **Applications → Applications → Create with provider** (en versiones nuevas: **New
   application**).
2. **Application**: **Name** `Litepod`, **Slug** `litepod` → **Next**.
3. **Provider type**: **OAuth2/OpenID Provider** → **Next**.
4. **Provider**:
   - **Authorization flow**: `default-provider-authorization-implicit-consent` (sin pantalla de
     consentimiento) o `...-explicit-consent`.
   - **Client type**: **Confidential**.
   - **Client ID** / **Client Secret**: cópialos ahora (los dos se generan solos).
   - **Redirect URIs**: modo **Strict**, `<base>/api/v1/auth/sso/callback`
     (ej. `https://litepod.tuempresa.com/api/v1/auth/sso/callback`).
   - **Signing Key**: `authentik Self-signed Certificate` (firma RS256).
   - **Advanced protocol settings → Scopes**: deja `openid`, `email`, `profile` (los default).
   - **Subject mode**: *Based on the User's hashed ID* (por defecto) está bien.
5. **Next** → (Bindings, opcional: limita quién puede usar la app a un grupo) → **Submit**.

### 2.4 Limitar quién entra (opcional)

**Applications → Litepod → Policy / Group / User Bindings → Bind existing policy/group/user** →
grupo `litepod-devs` (y `litepod-admins`). Sin bindings, cualquier usuario de Authentik puede
iniciar el login; Litepod igualmente rechaza correos fuera del dominio verificado.

### 2.5 Comprobar el token

**Applications → Providers → litepod → Preview** → elige el usuario `test`. El ID token debe
traer `email: test@tuempresa.com` y `groups: ["litepod-devs"]`.

El claim `groups` lo pone el mapping por defecto *authentik default OAuth Mapping: OpenID
'profile'*. Si lo quitaste, o quieres otro formato (IDs en vez de nombres), crea un **Scope
Mapping** en **Customization → Property Mappings**:

```python
return {"groups": [group.name for group in request.user.ak_groups.all()]}
```

## 3. En Litepod: crear la conexión

Antes: dominio verificado en Litepod ([sso.md → 3.1](sso.md#31-en-litepod)).

1. **Organización** → **Autenticación** → **Proveedores de identidad** → **Agregar proveedor de
   identidad**.
2. Rellena:

   | Campo | Valor |
   |---|---|
   | **Nombre** | `Authentik` |
   | **Protocolo** | **OpenID Connect** |
   | **URL del issuer** | `https://auth.tuempresa.com/application/o/litepod/` (con `/` final) |
   | **Client ID** | El del provider |
   | **Client secret** | El del provider |
   | **Claim de grupos** | `groups` |
   | **Rol base para miembros nuevos** | El mínimo |
   | **Crear cuentas en el primer inicio de sesión (just-in-time)** | Marcado |
   | **Activado** | Marcado |

3. **Guardar**.
4. **Editar** → copia la **URI de redirección** y compárala con **Redirect URIs** del provider:
   idénticas.

### 3.1 Grupos → roles

1. **Organización → Acceso**: crea `admins` y `devs` con sus roles.
2. **Autenticación → Mapeo de grupos**: `litepod-admins` → `admins`, `litepod-devs` → `devs`.
3. **Guardar**.

Detalle en [sso.md → 5](sso.md#5-grupos-y-roles).

## 4. Probar

1. Ventana privada → `<base>/login` → **Continuar con SSO** → `test@tuempresa.com`.
2. Login de Authentik → entra.
3. Vuelves a Litepod con sesión y en el grupo `devs`.

## 5. Si falla

Authentik registra cada intento en **Events → Logs**.

| Síntoma | Causa | Arreglo |
|---|---|---|
| Authentik: `Redirect URI Error` | URI no coincide | Copia la de Litepod en **Redirect URIs** |
| Litepod: `the ID token did not verify` | Issuer sin `/` final | Cópialo exacto de **OpenID Configuration Issuer** |
| Litepod: `could not read the identity provider's configuration` | Slug mal escrito, o la API no llega a Authentik | Abre `<issuer>.well-known/openid-configuration` desde la máquina de la API |
| Litepod: `the code exchange was refused` | Client secret incorrecto, o **Client type** Public | Confidential + secreto actual |
| Litepod: `the ID token carries no email address` | Usuario sin email o scope `email` quitado | 2.1 y 2.3 |
| Entra pero sin grupo | Mapping de `profile` quitado o grupos mapeados con otro nombre | 2.5 |
| Authentik: `Permission denied` | Hay bindings y el usuario no está en el grupo | 2.4 |

## Checklist

- [ ] Authentik con HTTPS y alcanzable desde la API de Litepod.
- [ ] Usuarios con email del dominio verificado.
- [ ] Provider OAuth2/OpenID, Confidential, redirect URI Strict exacta, signing key.
- [ ] Issuer copiado **con** `/` final.
- [ ] Preview del token con `email` y `groups`.
- [ ] Mapeo de grupos por **nombre**.
- [ ] Login de prueba OK.
