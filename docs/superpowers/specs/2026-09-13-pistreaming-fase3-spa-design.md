# PiStreaming — Spec de diseño: Fase 3 (SPA + endpoints faltantes)

**Fecha:** 2026-09-13
**Autor:** gustavo (con ANON)
**Estado:** aprobado en diseño sección por sección; pendiente de User Review Gate
**Base:** `docs/superpowers/specs/2026-09-13-pistreaming-design.md` (spec general) y `2026-09-13-pistreaming-fase2-playback-design.md`.

---

## 1. Objetivo

La Fase 2 dejó funcionando el núcleo: `POST /api/play` devuelve un `PlaybackPlan`, `GET /stream/:session`
sirve el media con `Range`, y el progreso se persiste vía `/api/progress/:type/:id`. Verificado en el
PiServer real (play + decode nativo). Pero el frontend es un `player.html` suelto y faltan los endpoints
de gestión (biblioteca, addons editables, ajustes) para que el proyecto sea usable de punta a punta.

**Fase 3 = la UI web propia completa (SPA vanilla embebida) + los endpoints que le faltan.**

## 2. Alcance

### Entra

- **SPA vanilla** (HTML/CSS/JS sin build de npm) embebida en el binario con `rust-embed`, con las vistas
  Buscar, Detalle, Reproductor, Biblioteca, Addons y Ajustes.
- **Biblioteca permanente** (`keep`): promueve el archivo cacheado de una sesión a `/data/library` y lo
  registra en una tabla propia; la limpieza LRU/TTL nunca lo toca.
- **Endpoints faltantes**: `GET/POST/DELETE /api/library`, `GET /library/:id/stream`,
  `PATCH /api/addons/:id`, `GET|PUT /api/settings`.
- **Estáticos**: `GET /` (index), `GET /assets/*`, `GET /icon.svg`.
- Tematización «cine oscuro — negro + rosa» y layout de pestañas ya definidos en el Visual Companion.

### No entra (fuera de alcance)

- Dockerfile multi-stage + `x-casaos` + instalación CasaOS → **Fase 4**.
- Transcodificación de video, VPN real, HLS, multi-usuario.
- Compatibilidad con el protocolo client-server de Stremio.
- Subtítulos.

---

## 3. Arquitectura de la SPA

### 3.1 Assets y embebido

Los assets viven en `crates/api/assets/`, junto al router que los sirve (donde hoy está `player.html`,
que se **elimina** en esta fase):

```
crates/api/assets/
  index.html    shell: barra superior con pestañas (Buscar · Biblioteca · Addons · Ajustes)
  app.js        router por hash + vistas
  style.css     tema «cine oscuro — negro + rosa»
  icon.svg      favicon
```

Se embeben con `rust-embed` en un módulo nuevo `crates/api/src/assets.rs`:

```rust
#[derive(RustEmbed)]
#[folder = "assets/"]
struct Assets;
```

- **En debug** lee del disco → editar `app.js` y recargar sin recompilar.
- **En release** embebe los bytes en el binario.

### 3.2 Rutas estáticas

| Ruta | Respuesta |
|---|---|
| `GET /` | `index.html` (`text/html`) |
| `GET /assets/*` | `app.js` / `style.css` con su `Content-Type` y `Cache-Control` |
| `GET /icon.svg` | el favicon |

### 3.3 Enrutado de vistas: hash-routing

La SPA navega por **hash** (`#/search`, `#/library`, `#/detail/:kind/:id`, `#/player/:session`,
`#/addons`, `#/settings`). El servidor **no** necesita fallback de rutas: todo entra por `/`. Menos
lógica en el router de axum y funciona igual sin importar el host desde el que se abra.

---

## 4. Modelo de datos: biblioteca + `keep`

### 4.1 Tabla nueva en `pistreaming-store`

Separada de `metadata_cache` (que es caché efímera y borrable):

```sql
CREATE TABLE library (
  id          TEXT PRIMARY KEY,   -- "<kind>:<id>"  p.ej. "movie:bbb-2014"
  kind        TEXT NOT NULL,      -- movie | series
  title       TEXT NOT NULL,
  file_path   TEXT NOT NULL,      -- /data/library/<id>.<ext>
  size_bytes  INTEGER NOT NULL,
  info_hash   TEXT,               -- para que la limpieza sepa qué NO tocar
  created_at  INTEGER NOT NULL
);
```

Notas: `library_dir` = `<data_dir>/library` (deriva de la config, no se hardcodea). El `id`
contiene `:` (p. ej. `movie:bbb-2014`): es legal como nombre de archivo en Linux y como segmento
de URL, así que se usa tal cual.

### 4.2 Flujo `keep`

1. Se resuelve el archivo de medios ya cacheado de la sesión (el `.mkv`/`.mp4` final). El
   `id`/`kind`/`title` de la ficha se toman de la metadata asociada a esa sesión (la misma que se
   envió en `POST /api/play`). Si la sesión no existe o la descarga no está completa → `409`.
2. Se crea el directorio `library_dir` (`/data/library`) si no existe.
3. **Hardlink** desde el archivo cacheado si es el mismo sistema de archivos; **copia** como fallback.
4. Se inserta la fila en `library` y se devuelve la ficha.

### 4.3 Interacción con la limpieza

Antes de borrar una sesión cacheada o una entrada de `metadata_cache`, la limpieza LRU/TTL consulta
`library` por `info_hash`/`file_path` y **excluye lo guardado**. Como el `keep` ya dejó el archivo en
`/data/library`, la caché original puede evictarse sin perder contenido.

### 4.4 Operaciones del store

- `keep(session) -> LibraryItem`
- `list_library() -> Vec<LibraryItem>`
- `remove_library(id) -> ()` (borra archivo + fila)

---

## 5. Endpoints (contratos)

### 5.1 Biblioteca

| Método y ruta | Body → Respuesta |
|---|---|
| `GET /api/library` | → `{"items":[{id,kind,title,file_path,size_bytes,created_at}]}` |
| `POST /api/library` | `{session}` → `201` + ficha (dispara el `keep`) |
| `DELETE /api/library/:id` | → `204` (borra archivo + fila; idempotente) |
| `GET /library/:id/stream` | `Range` → `206` + `accept-ranges`; `Content-Type` por extensión |

### 5.2 Addons

`PATCH /api/addons/:id` con body parcial `{enabled?: bool}` → `200` + addon actualizado
(activar/desactivar sin borrar). Se usa **el mismo identificador que ya usa `DELETE /api/addons/:url`**.

### 5.3 Ajustes

| Método | Contrato |
|---|---|
| `GET /api/settings` | valores efectivos + qué campo exige reinicio |
| `PUT /api/settings` | `{egress_bind?, cache_max_gb?, cache_ttl_hours?, data_dir?, http_port?}` → `{applied:[…], requires_restart:[…]}` |

- **Aplican en caliente:** `cache_max_gb`, `cache_ttl_hours` (el evictor los lee al vuelo).
- **Requieren reinicio:** `egress_bind`, `http_port`, `data_dir` (se persisten; la UI los marca).

### 5.4 Estáticos

`GET /` → `index.html`. `GET /assets/*`. `GET /icon.svg`.

### 5.5 Sin cambios

`POST /api/play`, `GET /stream/:session`, `GET|PUT /api/progress/:type/:id`. El reproductor de la SPA
sigue consumiendo `plan.playback_url` y `plan.progress_url`.

---

## 6. Flujo de la SPA y reglas de UI

### 6.1 Vistas

| Hash | Fuente de datos | Acciones |
|---|---|---|
| `#/search` (inicio) | `GET /api/search?q=` | grid de pósters; clic → detalle |
| `#/detail/:kind/:id` | `GET /api/meta/…` + `GET /api/streams/…` | **Reproducir** (`POST /api/play` → player) · **Guardar** (keep) |
| `#/player/:session` | `plan.playback_url` | `<video>`; guarda posición vía `plan.progress_url`; **Volver**; **Guardar** |
| `#/library` | `GET /api/library` | reproduce desde `/library/:id/stream` · **Quitar** (`DELETE`) |
| `#/addons` | `GET|POST /api/addons` | agregar (url) · activar/desactivar (`PATCH`) · borrar |
| `#/settings` | `GET|PUT /api/settings` | editar; aviso «requiere reinicio» en los campos que aplica |

### 6.2 Reglas de UI

- Tema «cine oscuro — negro + rosa» (fondo ~`#0b0d10`, superficie ~`#15181d`, texto ~`#e8eaed`,
  acento ~`#e11d48`); **los pósters mandan**; grid responsive.
- El reproductor es **vista propia en la misma pestaña** (no modal), con botón «Volver».
- Nav superior fijo con la pestaña activa resaltada.
- Estados explícitos: **cargando** (skeleton/spinner), **vacío**, **error** (mensaje + reintento).
- Los fallos de red muestran un banner no bloqueante y no rompen la navegación.
- Progreso: se guarda cada ~10 s y al salir del reproductor; al reabrir se restaura con
  `GET /api/progress/…`.

---

## 7. Manejo de errores y bordes

| Caso | Comportamiento |
|---|---|
| `POST /api/play` sin fuentes / torrent caído | La SPA muestra el error y **no** navega al player |
| `POST /api/library` con descarga incompleta | `409` + mensaje; UI: «Esperá a que termine la descarga» |
| `GET /library/:id/stream` id inexistente | `404`; rango inválido → `416` + `Content-Range` |
| Archivo de biblioteca borrado por fuera | `404`; la UI ofrece **Quitar** la ficha |
| `DELETE /api/library/:id` con archivo faltante | Borra la fila igual (idempotente) |
| `PUT /api/settings` con valor inválido | `400` indicando el campo |
| Hardlink entre dispositivos distintos | Fallback automático a copia |

---

## 8. Verificación

- **Unit (store):** CRUD de `library`, `keep` (hardlink vs copia), y que la limpieza LRU/TTL
  **excluya lo guardado**.
- **API (e2e_local, sin red):** `GET/POST/DELETE /api/library`, `PATCH /api/addons/:id`,
  `GET|PUT /api/settings`, y que `GET /` y `GET /assets/app.js` devuelvan `200` con su `Content-Type`.
- **Manual en el Pi:** abrir `http://192.168.100.100:8000/` desde la LAN → buscar → detalle →
  reproducir → guardar → biblioteca → reproducir local → ajustes.
- **Comandos:** `cargo test -p pistreaming-api -p pistreaming-store`; `cargo clippy`.
  **No** tocar `cargo fmt --check` (roto preexistente).

---

## 9. Riesgos y notas

- **`rust-embed` en debug lee del disco:** hay que verificar que el path relativo resuelva bien al
  ejecutar desde el workspace (`CARGO_MANIFEST_DIR`).
- **Hardlink cross-device:** `/data` puede estar en otro FS que el directorio de descarga; el fallback
  a copia debe cubrirlo explícitamente.
- **Sesión expirada al abrir `#/player/:session`:** la SPA debe degradar con error y ofrecer volver.
- No introducir build de npm ni dependencias de frontend: la SPA es HTML/CSS/JS plano.
