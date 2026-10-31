# Modo mantenimiento de una aplicación

Sirve una página de mantenimiento en **todos los dominios de una aplicación** mientras trabajas
en ella (una migración, un cambio delicado, un problema). Los visitantes ven la página; los
contenedores **siguen corriendo**.

Se configura en la aplicación: **Avanzado → Modo mantenimiento**.

## Cómo funciona

- Con el modo activo, Caddy responde con la página y un **HTTP 503** en cada dominio de la
  aplicación. No llega ningún tráfico a los contenedores.
- Al desactivarlo se restaura la ruta normal al instante. No hay reinicio ni redespliegue.
- Es independiente de cómo se construyó la aplicación: git, ZIP, imagen, Railpack o Compose.
- Mientras está activo puedes **desplegar** la aplicación. El despliegue cambia el contenedor
  de destino, no la página: los visitantes siguen viendo mantenimiento hasta que lo apagues.
- **La autenticación básica no se pide** durante el mantenimiento: todos ven la página.

## La página

Hay dos opciones:

- **Página por defecto**: un aviso sencillo ("We'll be right back / Volvemos en unos minutos").
- **HTML propio**: lo pegas, o cargas un archivo `.html`. Máximo **512 KiB**.

- **Sitio en ZIP**: un ZIP con un `index.html` en su raíz (o dentro de su única carpeta
  superior) y sus archivos: CSS, imágenes, fuentes. Hasta **10 MB y 500 archivos**.

La vista previa del HTML propio se muestra sin ejecutar scripts. Con tu propio HTML ten en cuenta:

- Es **un solo archivo**. Pon las imágenes y los estilos **dentro del HTML**, o usa el sitio en ZIP.
- Una página guardada con el modo apagado se conserva y se usa cuando lo actives.
- Guardar la página en blanco vuelve a la página por defecto.

### Sitio en ZIP

- El documento HTML responde **503** y lo que carga (CSS, imágenes, fuentes) responde **200**, porque
  el navegador no aplica una hoja de estilos ni dibuja una imagen que llega como error.
- Una ruta que no es un archivo (`/precios`) devuelve el `index.html`, para sitios de una sola
  página. Un archivo que no existe devuelve 404.
- Solo se sirve **mientras el modo está activo** y **solo a los dominios de la aplicación**. Nunca
  desde el origen de Litepod: el HTML de un usuario no puede ejecutarse donde vive tu sesión.
- Se sirve a través del propio API de Litepod. Si el API está caído un instante (por ejemplo,
  durante una actualización de Litepod), esas páginas no responden hasta que vuelva.
- Un ZIP inválido (sin `index.html`, rutas inseguras, demasiado grande) se rechaza y **no toca**
  el sitio que ya está sirviendo.
- Cada subida queda en el registro de auditoría como `maintenance.site_uploaded` o
  `maintenance.site_removed`.

## Encabezados de la respuesta

La respuesta lleva `Cache-Control: no-store`, `Retry-After: 300` y `X-Robots-Tag: noindex`, para
que ni los navegadores, ni los CDN, ni los buscadores se queden con una página temporal.

## Desde un agente (MCP) y el asistente

La herramienta `exec` tiene la acción `set_maintenance`: `application`, `enabled` (`true` o
`false`) y, si quieres, `page_html` (el documento HTML completo; en blanco vuelve a la página por
defecto). Por MCP se ejecuta como el usuario del token, que debe tener rol de editor en la
organización. En el chat del asistente es una propuesta que una persona confirma.

## Dónde se ve

Mientras está activo, el encabezado de la aplicación muestra la insignia **En mantenimiento**
junto al estado Running/Stopped.

## Límites actuales

- **Servidores con borde público**: aún no está disponible. Sus rutas las arma el script del
  propio servidor, que necesita su propio cambio. Activar el modo en una aplicación de ese tipo
  se rechaza con un mensaje claro en lugar de no hacer nada. Los servidores locales y los de
  relay privado sí funcionan.
- **ZIP con recursos** para la página: pendiente.

## Permisos y auditoría

Cambiar el modo o la página requiere poder editar la aplicación. Cada cambio queda en el registro
de auditoría como `maintenance.enabled`, `maintenance.disabled` o `maintenance.page_updated`
(sin guardar el contenido de la página).


## ZIP sites and Railpack

When you upload a ZIP, Litepod keeps a static copy and also compiles the same ZIP with Railpack in a
hidden application of its own (never listed, no domains, deleted with the application). Its
container is built and then **stopped, not removed**, so it uses nothing while maintenance is off.

- **Turning maintenance on** starts that container, waits until it answers its health check, and only
  then points your domains at it. The page is served with HTTP 503; its CSS and images are 200.
- **Turning it off** points the domains back at your application first, then stops the container.
- **If anything goes wrong** (build failed, container missing, health check not answering) visitors
  get the static copy of your ZIP instead, never a 502. If the container dies while maintenance is
  on, the domain answers the inline page.
- Compiled pages run on local servers only; on remote servers the static copy is served.
