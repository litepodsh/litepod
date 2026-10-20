[English](CONTRIBUTING.md) · **Español**

# Contribuir a Litepod

Gracias por querer ayudar. Litepod está actualmente en **modo estabilidad**: la prioridad es
robustez, corrección de bugs, tests y documentación. Las nuevas funcionalidades son bienvenidas
como ideas, pero **no está garantizado** que se implementen o fusionen mientras el proyecto se
estabiliza.

La versión corta: **abre un issue primero, espera a que lo acepten, y luego abre un PR.**

## El flujo (sin excepciones)

1. **Abre un issue.** Usa la plantilla de issue y elige **Bug report** o **Feature request** en
   el desplegable *Type*. Describe el problema, no la solución.
2. **Di si te encargas o no.** Comenta en el issue indicando si lo vas a implementar tú:
   ```
   Me encargo yo de este.
   ```
   Si no lo vas a implementar, también está bien — solo no abras un PR. En cualquier caso, un
   fix necesita un issue primero. Espera a que el mantenedor lo acepte (un 👍, una asignación,
   o un "adelante" explícito).
3. **Solo entonces abre un PR.** Un PR sin issue previo, o cuyo issue no fue aceptado, no será
   revisado. El PR debe referenciar su issue (p. ej. `Closes #123`).

Esta regla existe para evitar trabajo desperdiciado: muchas ideas no están alineadas con la
dirección actual y se declinan con amabilidad antes de que pierdas horas en ellas.

## Qué se fusiona (y qué no) ahora mismo

**Bienvenido, y probablemente se fusiona:**

- Corrección de bugs con una reproducción
- Tests (cobertura faltante, tests inestables)
- Correcciones de documentación y de traducciones
- Mejoras pequeñas y autocontenidas
- Subidas de dependencias que arreglan un problema real (CVE, build roto)

**Bienvenido como issue, pero no garantizado mientras esté en modo estabilidad:**

- Nuevas funcionalidades
- Nuevas integraciones, proveedores o plantillas
- Refactors o reescrituras de código que ya funciona
- Rediseños de la interfaz

Los feature requests no se rechazan — se registran y podrán programarse cuando el proyecto salga
del modo estabilidad. Abrir el issue pronto es como una idea entra en esa lista.

## Antes de enviar

- **Prueba localmente primero.** Levanta el issue, monta el stack y reproduce/verifica antes de
  empujar un PR. Mira [`apps/README.md`](apps/README.md) y [`apps/TESTING.md`](apps/TESTING.md)
  para ver cómo levantar el stack local.
- Ejecuta los checks de la parte que tocaste:
  ```sh
  cd apps
  just api-test          # tests de la API (Rust)
  just ui-test           # tests de la UI (SvelteKit)
  just fmt               # rustfmt + prettier
  ```
- Mantén los PRs pequeños y centrados en un solo issue. Un PR grande que mezcla un fix y una
  funcionalidad es más difícil de revisar y más probable que se decline.

## PRs

- Rellena la plantilla del pull request — sale rellenada cuando abres uno, y vive en
  [`.github/PULL_REQUEST_TEMPLATE.md`](.github/PULL_REQUEST_TEMPLATE.md).
- Referencia el issue (`Closes #123`).
- Enlaza cualquier discusión relacionada de los comentarios del issue.

El mantenedor revisa cada PR. Las revisiones las hace una sola persona y pueden tardar — si un
PR lleva un rato parado, un ping amable en el issue enlazado está bien.

## Código de conducta

Mira [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md). En resumen: nada de racismo, odio ni acoso, y
puedes escribir en **inglés o español, por igual**. El acoso, el spam y los comentarios
exigentes ("¿¿cuándo se va a mergear esto??") te llevan a bloqueo, no a merge.

## Seguridad

Si encontraste una vulnerabilidad, **no abras un issue ni un PR.** Sigue
[`SECURITY.md`](SECURITY.md).
