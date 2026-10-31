# Autenticación en dos pasos obligatoria

Cada persona puede activar la autenticación en dos pasos (2FA) en su cuenta, desde
**Cuenta → Seguridad**. Una organización puede ir un paso más allá y **exigirla a todos sus
miembros**. Funciona en self-hosted y en cloud.

## Qué hace

Con el requisito activo, un miembro que **no tiene 2FA** no puede usar la organización:
cualquier pantalla o llamada de esa organización responde con un error y la interfaz lo lleva a
activar el 2FA. Cuando termina, vuelve a donde estaba.

- Aplica a **todas las formas de entrar**: contraseña, SSO y OAuth.
- Aplica también a los **tokens de acceso** y a las conexiones MCP de esa persona. Un token no
  evita el requisito.
- Es **por organización**. Quien pertenece a varias solo es bloqueado en las que lo exigen.
- Está **desactivado por defecto**. Las organizaciones existentes no cambian hasta que un
  administrador lo active.

## Quién queda fuera

- El **administrador de la instancia** (SysAdmin) está exento. Es la salida de emergencia si
  alguien se queda sin acceso.
- Una persona que **no es miembro** sigue recibiendo el error de siempre ("no eres miembro"), sin
  enterarse de si la organización exige 2FA.

## Activarlo

1. Entra a **Organización → Seguridad** (solo la ven los administradores).
2. Activa **Exigir autenticación en dos pasos**.

Antes de activarlo, el panel te dice **cuántos miembros no tienen 2FA** todavía. A esos se les
negará el acceso hasta que lo activen, así que conviene avisarles antes.

**Primero necesitas 2FA en tu propia cuenta.** Si eres administrador y no lo tienes, el interruptor
queda bloqueado y la API lo rechaza: de lo contrario te quedarías fuera de la pantalla que sirve
para desactivarlo. El SysAdmin no necesita esto porque está exento.

## Ver quién lo tiene

En **Organización → Personas**, quienes pueden gestionar miembros ven una etiqueta junto a cada
persona: **2FA activo** o **Sin 2FA**. Los demás miembros no ven esa información de sus
compañeros.

## Lo que ve un miembro bloqueado

Al abrir la organización lo llevamos a **Cuenta → Seguridad** con un aviso. Escanea el QR, confirma
el código y guarda los códigos de recuperación. Al cerrar ese cuadro, lo devolvemos a la pantalla
donde estaba.

## Si alguien pierde su segundo factor

Se resuelve con sus **códigos de recuperación**. Si no los tiene, el administrador de la
instancia puede borrar su 2FA (la persona vuelve a quedar sin 2FA y tendrá que activarlo de nuevo
para entrar a organizaciones que lo exigen):

```sh
podman exec -it litepod-api litepod-api reset-2fa --email persona@empresa.com
```

## Auditoría

Cada cambio del requisito queda en el registro de auditoría de la organización como
`organization.two_factor_requirement_changed`, con el valor nuevo (`required: true/false`).
