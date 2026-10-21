# Roadmap

[English](ROADMAP.md) · **Español**

Litepod está en **modo estabilidad**. El foco actual es que lo que ya existe sea confiable, no
agregar funcionalidades nuevas.

## Foco actual

### 1. Despliegues estables

Los despliegues deben terminar bien o fallar de forma predecible, con logs y estado claros.

- Corregir despliegues que se quedan colgados, fallan sin avisar o terminan en un estado incorrecto.
- Recuperación limpia tras un despliegue fallido o interrumpido (rollback, reintentos, contenedores sobrantes).
- Builds y redespliegues confiables para todos los tipos de origen (Git, Compose, plantillas, sitios estáticos).
- Más tests sobre el flujo de despliegue.

### 2. Podman rootless

Podman rootless debe funcionar igual de bien que con root, en todas las distribuciones soportadas.

- Corregir las diferencias entre instalaciones con root y rootless (red, puertos, volúmenes, permisos).
- Hacer confiables el instalador y las actualizaciones en modo rootless.
- Documentar los límites de rootless y cómo resolverlos.

### 3. MCP y asistencia con agentes

El endpoint MCP permite que un agente (Claude, Cursor, …) consulte y opere Litepod. El objetivo
es que sea estable y confiable, no agregar más herramientas.

- Que `get` y `exec` devuelvan datos consistentes y correctos.
- Que las propuestas (acciones de `exec` que confirma una persona) funcionen de principio a fin, hasta el despliegue.
- Errores claros con los que el agente pueda actuar.
- Más tests sobre las herramientas MCP.

## Solicitudes de funcionalidades

Las funcionalidades nuevas no están en el roadmap mientras el proyecto se estabiliza. Si quieres
alguna, abre un issue en este repositorio: [github.com/litepodsh/litepod/issues](https://github.com/litepodsh/litepod/issues).
Elige **Feature request** en la plantilla del issue. Las solicitudes quedan registradas y pueden
planificarse cuando el proyecto salga del modo estabilidad. Consulta [CONTRIBUTING.es.md](CONTRIBUTING.es.md) para el flujo completo.
