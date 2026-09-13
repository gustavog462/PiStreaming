# PiStreaming — Fase 2: Playback real (torrent → media → stream)

**Fecha:** 2026-09-13
**Autor:** gustavo (con ANON)
**Estado:** aprobado en diseño, pendiente de revisión final del spec
**Base:** `fase1-fundacion` @ `c5e5d02` (33 tests verdes, clippy limpio)

---

## 1. Objetivo

Cerrar el corte vertical que la Fase 1 dejó abierto: **reproducir de verdad** un torrent,
de punta a punta, en el navegador del Pi. Fase 1 dejó el esqueleto (addons, búsqueda, meta,
streams, config, store); la Fase 2 agrega el motor de torrents, el motor de media y el
servidor de bytes, y los une con `POST /api/play` + `GET /stream/:session`, más progreso
y un player mínimo.

Regla de done (spec §11): `cargo test` verde **y** un play real en el navegador del Pi.

---

## 2. Alcance

### Entra en Fase 2

- **`torrent`** — motor real con `librqbit` 9.x: `add_magnet`, espera de metadata,
  prioridad secuencial (ya es el default de rqbit), lectura por `FileStream`, stats.
- **`media`** — `ffprobe` (JSON) sobre el stream, `decide(probe) → PlaybackPlan`,
  `remux` (ffmpeg `-c copy`, MKV→fMP4) y `recode_audio` (AC3/EAC3/TrueHD/DTS→AAC, solo audio).
- **Playback** — `POST /api/play` (crea sesión) y `GET /stream/:session` con soporte **Range**.
- **Progreso** — `GET/PUT /api/progress/:type/:id`.
- **Eviction LRU** de la caché de disco (`CACHE_MAX_GB` / `CACHE_TTL_HOURS`).
- **Player mínimo** — página `/play/:session` con `<video>` (JS vanilla, embebido).

### NO entra (queda para Fase 3+)

- `keep` → biblioteca (`/data/library`) y `GET /api/library`.
- SPA completa del spec §9 (gestión de addons, detalle, settings).
- `PATCH /api/addons/:id` (deuda m17 del review de Fase 1).
- Docker/CasaOS, VPN/egress real, HLS, transcodificación de video.

---

## 3. Decisiones cerradas de Fase 2

| Tema | Decisión | Motivo |
|---|---|---|
| UI | **Con player mínimo** (`<video>` + fallback a URL cruda); SPA completa a Fase 3 | respuesta del usuario |
| Validación | **Dev local** (unit + integración determinista) y **play real en el Pi** como checkpoint manual final | respuesta del usuario |
| `EGRESS_BIND` | **Configurable, bind real diferido**: se acepta/valida/loguea; `bind_device_name` solo si el valor es válido, si no `None` + `warn` | respuesta del usuario; alineado al P0 del spec |
| Serving | **A — stream directo de librqbit + remux incremental** (`FileStream` → `Body::from_stream`; remux a fMP4 cuando haga falta) | respuesta del usuario; spec §6/§7 |
| Caché | **Eviction LRU** con tope/TTL, **sin** `keep`/biblioteca | respuesta del usuario |
| HLS | Descartado en Fase 2 | simplicidad; el spec lo deja como alternativa |

---

## 4. Componentes y cambios por crate

### `core`

- Tipos nuevos: `PlaySession`, `SessionId` (string opaco, ULID/uuid v4), `PlaybackPlan`
  (ya esbozado en el spec §4) con variantes `Direct` / `Remux { browser_may_fail }` /
  `RecodeAudio` / `Unsupported`.
- Traits ya previstos: `TorrentEngine`, `MediaEngine`, `Store` — se completan las firmas.
- Errores: `CoreError` existente (`{error, code}`); se agregan casos (metadata timeout,
  torrent stalled, ffmpeg ausente, sesión no encontrada) mapeados a 404/500/502/504.

### `torrent` (hoy scaffold)

- `LibrqbitEngine`:
  - Una **`Session` única** de larga vida (creada en `server` boot), con
    `SessionOptions.bind_device_name = Some(cfg.egress_bind)` si el valor fue configurado y
    validado; si no, `None` (+ `warn!`). La descarga secuencial es el default de rqbit.
  - `add_magnet(magnet)`: `AddTorrentOptions { output_folder: /data/cache, overwrite: true, ... }`.
  - `wait_metadata(timeout = 30 s)` → error 504 si no resuelve.
  - Selección de archivo: el archivo de video más grande (`only_files` / lista de files).
  - `stream(file_id) -> FileStream` (bloquea hasta tener la pieza; lookahead 32 MB).
  - `status(info_hash) -> TorrentStatus` (`Downloading` / `ReadyMetadata` / `Stalled` / `Seeding`)
    a partir de `TorrentStats`.
  - `egress`: solo log/validación en Fase 2.

### `media` (hoy scaffold)

- `probe(file) -> Probe`: `ffprobe -v quiet -print_format json -show_format -show_streams`.
  Fuente: el `FileStream` del torrent vía pipe (no espera la descarga completa).
- `decide(probe) -> PlaybackPlan`: ver §6.
- `remux(stream, out_path)`: `ffmpeg -c copy -movflags frag_keyframe+empty_moov` → `.mp4` en
  la carpeta de caché del infohash.
- `recode_audio(stream, out_path)`: `ffmpeg -c:v copy -c:a aac` (única recodificación permitida).
- Detección de binario: si `ffmpeg`/`ffprobe` no están en `PATH` → error claro (500).

### `api`

- `PlaySessionRegistry` en memoria: `Arc<Mutex<HashMap<SessionId, PlaySession>>>`.
- Rutas nuevas: `POST /api/play`, `GET /stream/:session`,
  `GET /api/progress/:type/:id`, `PUT /api/progress/:type/:id`, `GET /play/:session`.
  - **Divergencia consciente con el spec base:** §8 del spec define `/api/progress/:id`,
    pero la clave de progreso es `(type, id)` (§5). Se refina a `/api/progress/:type/:id`
    para no inventar un id compuesto. Anotar al actualizar el spec base en Fase 3.
- `GET /stream/:session`:
  - **Direct**: responde el `FileStream` con `Range`; `Content-Length` = tamaño del archivo
    (conocido por metadata). Usa `Body::from_stream` (`tokio_util::io::ReaderStream`).
  - **Remux/RecodeAudio**: ffmpeg lee del `FileStream` y escribe fMP4 en caché; se sirve ese
    archivo creciente (fMP4 fragmentado ⇒ sin `Content-Length` total; 206 por chunks).
  - Seek: direct = nativo (el `FileStream` soporta seek por pieza); remux MVP = si el seek
    cae fuera de lo ya disponible, reiniciar el tramo con `-ss`.
  - 404 con `{error,code}` si la sesión no existe; 416 si el `Range` es inválido.

### `store`

- Tabla/métodos de progreso: `(type, id, position, duration, updated_at)`, con UPSERT.
- (Config ya existe en Fase 1.)

### `server`

- Wiring config → `LibrqbitEngine` + `MediaEngine` + registry; inyección en `AppState`.
- **Job tokio de eviction** periódico (intervalo configurable, p. ej. cada 15 min).

---

## 5. Flujo de datos (play)

```
POST /api/play { stream }
  → torrent.add_magnet(magnet)
  → torrent.wait_metadata(30s)
  → elegir archivo de video (el más grande)
  → media.probe(FileStream)          # ffprobe por pipe
  → media.decide(probe) → PlaybackPlan
  → registry.insert(session)
  ← 200 { session, plan, url: "/stream/<session>" }

GET /stream/<session>  (Range)
  → direct:        Body::from_stream(FileStream)      # Range nativo
  → remux/recode:  ffmpeg(FileStream) → /data/cache/<hash>/<id>.mp4  # servir creciente
```

Progreso: el player hace `PUT /api/progress/:type/:id` con throttle (~10 s) y al pausar/cerrar.

---

## 6. `media::decide` (tabla)

| Contenedor | Video | Audio | Plan |
|---|---|---|---|
| MP4/MOV/WebM/M4V | H.264/VP8/VP9/AV1 | AAC/Opus/Vorbis | **Direct** |
| MKV/AVI/TS | H.264/HEVC/AV1 | AAC/Opus/Vorbis | **Remux** (fMP4) |
| cualquiera | HEVC/AV1 | — | **Remux** con `browser_may_fail=true` (+ URL cruda expuesta) |
| cualquiera | H.264/HEVC/AV1 | AC3/EAC3/TrueHD/DTS | **RecodeAudio** (`-c:v copy -c:a aac`) |
| cualquiera | MPEG2/VC1/otro | — | **Unsupported** (error claro, sin transcodificar) |

> Regla dura del spec: el **video nunca se transcodifica**; la única recodificación admitida es
> audio a AAC.

---

## 7. Caché y eviction

- Layout: `/data/cache/<infohash>/<archivo>`.
- Job periódico: si el total supera `CACHE_MAX_GB`, o un entry supera `CACHE_TTL_HOURS`,
  se borran los más antiguos (por mtime/último uso) que **no estén en una sesión activa**.
- En Fase 2 no hay `kept`; todos los entries son evictables cuando quedan inactivos.
- Los temporales de remux viven dentro de la carpeta del infohash y se borran con él.

---

## 8. Player mínimo

- Ruta: `/play/:session` (HTML+JS vanilla embebido con `rust-embed`).
- `<video controls autoplay>` con `<source src="/stream/:session">`.
- Si `plan.browser_may_fail`, muestra un aviso y un enlace "abrir URL cruda".
- Reporta progreso por `fetch` con throttle (~10 s) + en `pause`/`beforeunload`.
- Sin framework, sin build step.

---

## 9. Errores (contrato)

| Caso | HTTP | Body |
|---|---|---|
| Sesión inexistente | 404 | `{ "error": "...", "code": 404 }` |
| `Range` inválido | 416 | `{ "error": "...", "code": 416 }` |
| Metadata no resuelta en 30 s | 504 | `{ "error": "...", "code": 504 }` |
| Torrent stalled | 502/504 | `{ "error": "...", "code": 502 }` |
| ffmpeg/ffprobe ausente | 500 | `{ "error": "...", "code": 500 }` |
| Codec no soportado | 422 | `{ "error": "...", "code": 422 }` |

---

## 10. Testing y verificación

- **Unit** (local, sin red):
  - tabla `decide` completa (todas las filas de §6);
  - parser de `Range` (`bytes=0-`, `bytes=100-200`, `bytes=-500`, inválidos);
  - `PlaySessionRegistry` (insert/get/remove, sesión inexistente);
  - selección de eviction (por tope y por TTL, excluyendo activos);
  - `status` mapping desde `TorrentStats`.
- **Integración determinista (local, sin red pública)**: un test crea un torrent desde un
  archivo local de prueba y levanta una `Session` **seeder local**; reproduce vía API y valida
  que el stream devuelve los bytes correctos (incluido un `Range` parcial).
- Los tests que dependen de `ffmpeg`/`ffprobe` se **saltan o marcan** si el binario no está en `PATH`.
- **TDD estricto** (rojo→verde) en cada task del plan.
- **Gate**: `cargo test` verde + `cargo clippy --workspace --all-targets -- -D warnings` limpio.
- **E2E manual**: play real en el navegador del Pi (checkpoint del usuario).

---

## 11. Riesgos y preguntas abiertas

- **Firmas finas de `librqbit`** (métodos exactos de `FileStream`, campos de `TorrentStats`)
  quedaron pendientes en el research; se confirman al implementar la primera task del crate.
- **Range sobre archivo creciente (remux)**: sin `Content-Length` total; hay que verificar
  que el `<video>` del navegador tolere el fMP4 fragmentado servido por chunks.
- **ffprobe por pipe sobre `FileStream`**: confirmar que ffprobe no requiere `seek` sobre el
  stream (el `FileStream` puede necesitar descargar piezas adelantadas).
- **Egress bind**: en Fase 2 es config/validación; `bind_device_name` real se activa en la fase VPN.

---

## 12. Deuda heredada de Fase 1 que NO se toca aquí

- m7 (inversión de capas `addons→store`), m13 (`std::sync::Mutex<Connection>` + `unwrap` → poison),
  m16 (deps sin usar), m17 (`PATCH /api/addons/:id`), m18 (TraceLayer/CORS/body-limit/graceful shutdown),
  NITs varios. Se retoman en Fase 3 salvo que una task de Fase 2 los toque por cercanía.
