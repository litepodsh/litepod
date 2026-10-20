[English](GOVERNANCE.md) · **Español**

# Gobernanza

Este documento explica quién toma decisiones en Litepod y cómo. Es simple a propósito: esto es
un proyecto pequeño, no una empresa.

## Modelo

Litepod usa un modelo de **un solo mantenedor (BDFL)**. Hoy hay un mantenedor que tiene la
última palabra en todo. A medida que el proyecto crezca, esto puede cambiar — ver
[Roles](#roles).

## Roles

### Mantenedor

- **Quién:** [@Sebasmzt](https://github.com/Sebasmzt) — actualmente el único mantenedor.
- **Poderes:** la única persona con permiso de merge. Revisa cada pull request. Toma las
  decisiones grandes (dirección, cambios que rompen compatibilidad, releases).
- **Responsabilidad:** mantener el proyecto sano y honesto.

### Contribuidor

- **Quién:** cualquiera que abra issues o pull requests.
- **Poderes:** proponer cambios, reportar bugs, pedir funcionalidades. Sin permiso de merge.

### Roles futuros

Cuando contribuidores de confianza demuestren su valía con el tiempo, el mantenedor puede
promoverlos a:

- **Triager** — gestionar y etiquetar issues, ayudar a reproducir bugs.
- **Mantenedor** — hacer merge de pull requests y compartir decisiones.

No hay plazo ni promesa para esto. Ocurre cuando ocurre, según la confianza y el historial.

## Cómo se toman las decisiones

- **Cosas pequeñas** (corrección de bugs, docs, tests, mejoras menores) se discuten en su issue
  o PR y las mergea el mantenedor.
- **Cosas grandes** (nuevas funcionalidades, cambios que rompen compatibilidad, dirección del
  proyecto, releases) las decide el mantenedor. Los contribuidores las proponen primero como
  issues — ver [`CONTRIBUTING.md`](CONTRIBUTING.md).
- El mantenedor puede declinar o posponer cualquier cosa. Actualmente el proyecto está en
  **modo estabilidad**: las nuevas funcionalidades no están garantizadas mientras se estabiliza.

## Revisiones

El mantenedor revisa cada PR. Las revisiones las hace una sola persona y pueden tardar.

## Si el mantenedor desaparece

Litepod está bajo **Apache 2.0** (excepto `ee/`, bajo la Enterprise License), así que el plan de
respaldo es simple y siempre está disponible: **cualquiera puede hacer fork del proyecto y
continuarlo**. Sin necesidad de permiso.

Para que eso sea más fácil, el mantenedor guarda una lista privada de "bus factor" (dominio,
credenciales de publicación, acceso a servidores) para que una persona de confianza pueda tomar
el relevo si hace falta. Esa lista, a propósito, no está en el repositorio público.

## Cambiar este documento

Solo el mantenedor puede cambiar este documento, abriendo un pull request y haciéndole merge.
