# SAML gratis: probar y configurar SAML con Keycloak

[← SSO: lo común](sso.md) · SAML con Entra ID: [saml.md](saml.md) · OIDC con Keycloak: [keycloak.md](keycloak.md)

## Con qué se puede hacer SAML gratis

| IdP | Coste | Dónde corre | SAML para Litepod | Notas |
|---|---|---|---|---|
| **Keycloak** | Gratis, open source | Self-hosted | Sí | **Recomendado.** Esta guía |
| **Authentik** | Gratis, open source | Self-hosted | Sí (*SAML Provider*) | Mismo flujo que aquí, otros nombres |
| **Entra ID Free** | Gratis | Nube (Microsoft) | Sí, asignando usuarios sueltos | Asignar grupos pide P1/P2. Guía: [saml.md](saml.md) |
| Logto | Gratis self-hosted | Self-hosted | No práctico | Usa OIDC ([logto.md](logto.md)) |
| Google Workspace | De pago | Nube | Sí | No es gratis |

Elegimos **Keycloak**: gratis, sin cuenta, corre en Podman en tu máquina o servidor, y manda
email y grupos sin licencias.

Recuerda: **SAML y OIDC hacen lo mismo** (login). Si el IdP soporta los dos, usa OIDC. SAML es
para cuando el cliente o su equipo de seguridad lo exige.

## Requisitos en Litepod

- Build con SAML: las imágenes de release lo traen. En local: `cargo build --features saml`
  (necesita `libxml2` y `xmlsec1`). Si en **Agregar proveedor de identidad** no aparece el
  selector **Protocolo**, tu build no tiene SAML.
- Dominio verificado en Litepod ([sso.md → 3.1](sso.md#31-en-litepod)).
- Si hay una conexión OIDC activa para el mismo dominio, desactívala: **una conexión activa por
  dominio**.

## Qué se intercambia

| Dato | Lo da | Se pega en | Nombre en Keycloak |
|---|---|---|---|
| **XML de metadatos del IdP** | Keycloak | Litepod | `https://<host>/realms/<realm>/protocol/saml/descriptor` |
| **Identificador (Entity ID)** | Litepod | Keycloak | **Client ID** del client SAML |
| **URL de ACS** | Litepod | Keycloak | **Assertion Consumer Service POST Binding URL** |
| **Claim de grupos** | Tú eliges | Los dos | Mapper *Group list*, atributo `groups` |

Orden: con Keycloak el XML del IdP no depende del client, así que **primero** se crea la conexión
en Litepod (da el Entity ID) y **después** el client en Keycloak con ese Entity ID. Sin valores
provisionales.

## 1. Levantar Keycloak

Prueba rápida:

```bash
podman run -d --name keycloak -p 8081:8080 \
  -e KC_BOOTSTRAP_ADMIN_USERNAME=admin \
  -e KC_BOOTSTRAP_ADMIN_PASSWORD=admin \
  quay.io/keycloak/keycloak:26.0 start-dev
```

Producción (Postgres + HTTPS detrás de Caddy): ver [keycloak.md → 1.2](keycloak.md#12-producción-detrás-de-caddy-con-postgres).

En la consola (http://localhost:8081, `admin`/`admin`):

1. **Create realm** → `tuempresa`.
2. **Users → Add user**: `test`, **Email** `test@tuempresa.com`, **Email verified** On →
   **Create** → **Credentials → Set password**, **Temporary** Off.
3. **Groups → Create group**: `litepod-admins`, `litepod-devs`. Mete a `test` en `litepod-devs`.

## 2. Bajar el metadata del IdP

Abre y guarda el XML:

```bash
curl -o keycloak-idp.xml http://localhost:8081/realms/tuempresa/protocol/saml/descriptor
```

Trae el certificado de firma y el endpoint HTTP-Redirect, que es lo que Litepod exige.

## 3. En Litepod: crear la conexión SAML

1. **Organización** → **Autenticación** → **Agregar proveedor de identidad**.
2. Rellena:

   | Campo | Valor |
   |---|---|
   | **Nombre** | `Keycloak SAML` |
   | **Protocolo** | **SAML 2.0** |
   | **XML de metadatos de federación del IdP** | Contenido completo de `keycloak-idp.xml` |
   | **Claim de grupos** | `groups` |
   | **Rol base para miembros nuevos** | El mínimo |
   | **Crear cuentas en el primer inicio de sesión (just-in-time)** | Marcado |
   | **Activado** | Marcado |

3. **Guardar** → **Editar**. Copia:
   - **Identificador (Entity ID)**: `<base>/api/v1/auth/saml/<connection_id>/metadata`
   - **URL de ACS**: `<base>/api/v1/auth/saml/acs`

El Entity ID también es una URL que sirve el metadata del SP: puedes abrirla para comprobarlo.

## 4. En Keycloak: crear el client SAML

### 4.1 Client

1. **Clients → Create client**.
2. **Client type**: **SAML**. **Client ID**: el **Identificador (Entity ID)** de Litepod,
   exacto. **Name**: `Litepod` → **Next**.
3. **Login settings**:
   - **Valid redirect URIs**: `<base>/api/v1/auth/saml/acs`
   - **Master SAML Processing URL**: vacío.
   → **Save**.
4. Pestaña **Settings**, bloque **SAML capabilities**:
   - **Name ID format**: `persistent` (ID estable; el email va aparte en un atributo).
   - **Force name ID format**: On.
5. Bloque **Signature and Encryption**:
   - **Sign documents**: On.
   - **Sign assertions**: On.
   - **Signature algorithm**: `RSA_SHA256` (Litepod rechaza SHA-1).
6. **Save**.
7. Pestaña **Keys** → **Client signature required**: **Off** → confirmar.
   Litepod no firma sus AuthnRequest; con esto en On, Keycloak responde
   `Invalid requester`.
8. Pestaña **Advanced** → **Assertion Consumer Service POST Binding URL**:
   `<base>/api/v1/auth/saml/acs` → **Save**.

No actives **Encrypt assertions**: Litepod no descifra assertions.

### 4.2 Mappers: email y grupos

**Clients → Litepod → Client scopes →** `<client-id>-dedicated` **→ Configure a new mapper**:

1. **User Property**:
   - **Name**: `email`
   - **Property**: `email`
   - **SAML Attribute Name**: `email`
   - **SAML Attribute NameFormat**: `Basic`
   → **Save**.
2. Otra vez **Add mapper → By configuration → Group list**:
   - **Name**: `groups`
   - **Group attribute name**: `groups`
   - **SAML Attribute NameFormat**: `Basic`
   - **Single Group Attribute**: On
   - **Full group path**: Off
   → **Save**.

Litepod busca el email en los atributos `email`, `mail`,
`http://schemas.xmlsoap.org/ws/2005/05/identity/claims/emailaddress` o
`urn:oid:0.9.2342.19200300.100.1.3`; si no hay, usa el NameID si parece un correo.

## 5. Grupos → roles

1. **Organización → Acceso**: crea `admins` y `devs` con sus roles.
2. **Autenticación → Mapeo de grupos**: `litepod-admins` → `admins`, `litepod-devs` → `devs`
   (por **nombre**: Keycloak manda nombres).
3. **Guardar**.

## 6. Probar

1. Ventana privada → `<base>/login` → **Continuar con SSO** → `test@tuempresa.com`.
2. Login de Keycloak → entra.
3. Vuelves a Litepod con sesión y en el grupo `devs`.

Solo **SP-initiated**: se entra desde Litepod, no desde el portal de cuentas de Keycloak.

Para ver la assertion: extensión de navegador *SAML-tracer* (Firefox/Chrome), o
**Realm settings → Events** en Keycloak.

## 7. Si falla

| Síntoma | Causa | Arreglo |
|---|---|---|
| Keycloak: `Invalid requester` | **Client signature required** en On, o Client ID ≠ Entity ID | 4.1 pasos 2 y 7 |
| Keycloak: `Invalid redirect uri` | ACS no está en **Valid redirect URIs** | 4.1 paso 3 |
| Litepod rechaza el metadata: `no signing certificate` | XML incompleto o de otro endpoint | Usa `/protocol/saml/descriptor` del realm |
| Litepod rechaza el metadata: `no HTTP-Redirect single sign-on endpoint` | Metadata recortado | Pega el XML completo |
| Litepod rechaza la respuesta | Firma SHA-1, assertion cifrada, o audience/destination no coinciden | 4.1 pasos 5 y 8; no cifrar |
| `the assertion carries no email address` | Falta el mapper `email` y el NameID no es un correo | 4.2 |
| Entra pero sin grupo | Falta *Group list*, o *Full group path* On (`/litepod-devs`) | 4.2 |
| No se puede activar la conexión | Otra conexión activa en el dominio, o dominio sin verificar | Desactiva la otra / verifica |

## Checklist

- [ ] Build de Litepod con SAML (aparece **Protocolo**).
- [ ] Conexión SAML en Litepod con el XML de `/protocol/saml/descriptor`.
- [ ] Client SAML con **Client ID** = Entity ID de Litepod.
- [ ] Sign documents + assertions, RSA_SHA256, **Client signature required** Off.
- [ ] ACS POST Binding URL = `<base>/api/v1/auth/saml/acs`.
- [ ] Mappers `email` y `groups` (Full group path Off).
- [ ] Login de prueba OK.
