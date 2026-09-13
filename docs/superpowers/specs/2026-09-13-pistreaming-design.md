# PiStreaming — Spec de diseño

**Fecha:** 2026-09-13
**Autor:** gustavo (con ANON)
**Estado:** aprobado en diseño, pendiente de revisión final del spec

---

## 1. Objetivo

Un server self-hosted tipo Stremio, escrito en Rust, que corre headless en el PiServer
(Raspberry Pi 4B, 4 GB RAM, arm64, Ubuntu 24.04) y recibe tráfico de cualquier dispositivo
de la red (navegador de escritorio, celu, TV) sirviendo películas/series desde torrents,
elegidas vía addons compatibles con el protocolo de Stremio (Torrentio, Cinemeta, etc.).

Dos motivaciones combinadas:

1. **Función** — un backend nativo, liviano y estable que no dependa de Node/Electron.
2. **Craft** — construir el núcleo en Rust es el punto: control nativo propio.

---

## 2. Alcance

### v1 (entra)

- Implementar **solo el protocolo de addons de Stremio** como cliente:
  `manifest.json`, `/catalog`, `/meta`, `/stream`.
  Consume addons existentes; frontend y cliente son propios.
  (Subtítulos: fuera de v1, va al roadmap.)
- Motor de torrents con **streaming secuencial** (ver mientras baja).
- UI web mínima propia (gestión de addons, búsqueda, detalle, reproductor, biblioteca).
- Streaming + modelo **S2** de almacenamiento (caché de disco con tope + `keep` opcional).
- Deploy como **app Docker administrada por CasaOS**, mismo molde que StreamVault.

### No-objetivos de v1 (el norte)

- **Compatibilidad con el protocolo client-server de Stremio** para que las apps oficiales
  (celu/TV) apunten al server. Es el norte, pero NO debe ensuciar el diseño de v1:
  se deja la puerta abierta (API limpia y versionada), no se implementa.
- Transcodificación de video (el Pi no tiene encoder → imposible por hardware).
- Multi-usuario / cuentas. v1 es single-user, pensado para red local + Tailscale.
- Addon *server* (servir el propio catálogo a terceros). Solo cliente de addons en v1.

---

## 3. Decisiones cerradas

| Tema | Decisión | Motivo |
|---|---|---|
| Forma | Server headless que streamea a la red + construir en Rust | respuesta dj "la a y la c" |
| v1 vs norte | v1 = solo protocolo de addons (cliente). Norte = compat client-server Stremio | "1 como v1, 2 como norte" |
| Storage | S2 — streaming + `keep` (biblioteca local opcional), buffer SIEMPRE a disco | "2"; 4 GB RAM no aguantan 4K en RAM |
| VPN | P0 — sin VPN, pero **interfaz de salida configurable desde el día uno** | "p0"; pasar a VPN con kill-switch sin refactor |
| Enfoque técnico | B — núcleo nativo propio; solo se prestan protocolos. `librqbit` + `axum` + ffmpeg subprocess. `stremio-core` = referencia, no dependencia | "la B" |
| Deploy | Docker single-container como app custom de CasaOS (molde StreamVault) | pedido dj |
| Nombre | **PiStreaming** | dj |

---

## 4. Arquitectura — workspace Rust

Crates de un solo propósito, cada uno entendible sin leer sus internals:

```
PiStreaming/
├── Cargo.toml            # workspace
├── crates/
│   ├── core/             # modelos + traits, sin I/O
│   ├── addons/           # cliente del protocolo de addons
│   ├── torrent/          # wrapper sobre librqbit
│   ├── media/            # ffprobe + decisión + pipeline remux + servidor bytes
│   ├── store/            # persistencia SQLite
│   ├── api/              # axum: REST + streaming + UI estática
│   └── server/           # binario: config + wiring + tareas supervisadas
├── Dockerfile
├── docker-compose.yml    # con x-casaos
├── .env.example
└── README.md
```

### `core` — modelos y contratos

- `Manifest` (id, version, name, resources, types, catalogs, idPrefixes).
- `CatalogEntry`, `CatalogRequest { type, id, extra }`.
- `MetaItem` (preview de catálogo), `MetaDetail` (ficha completa).
- `Stream` (infoHash/magnet/url, name, title, calidad inferida, seeds, size, addon fuente).
- `PlaybackPlan` (codec video/audio, ruta elegida `direct`|`remux`, `playback_url`,
  flags `needs_recode_audio`, `browser_may_fail`).
- Traits: `AddonClient`, `TorrentEngine`, `MediaEngine`, `Store`. Todo lo pesado
  (red, disco, procesos) detrás de traits para poder mockear en tests.

### `addons` — cliente del protocolo

- `reqwest` + `serde`. Fetch de `/manifest.json`, cacheado.
- `search(type, query)` → consulta los catálogos de búsqueda de addons habilitados en
  paralelo, con timeout por request (~10 s) y reintentos con backoff.
- `meta(type, id)`, `streams(type, id)`.
- **Aislamiento**: un addon caído o lento no rompe la búsqueda global; se devuelve lo que
  haya y se marca el addon como degradado (`HttpError.retryable` clasificado).

### `torrent` — wrapper librqbit

- `add_magnet(magnet, egress_bind)` → crea sesión, resuelve metadata.
- **Modo streaming**: prioriza piezas en orden secuencial + primeras/últimas.
- **Egress configurable**: `bind` a una interfaz (`eth0`, `wg0`, o netns futura). P0 = `eth0`.
- Tope de caché (`CACHE_MAX_GB`) y limpieza LRU por antigüedad (`CACHE_TTL_HOURS`),
  nunca toca items `kept`.
- Emite estado (`downloading`, `ready_metadata`, `stalled`, `seeding`) por sesión.

### `media` — ffprobe + remux

- `probe(path)` vía `ffprobe -v quiet -print_format json -show_format -show_streams`.
- `decide(probe) -> PlaybackPlan` (regla fina, §7).
- `remux(path) -> url` — `ffmpeg -c copy` (MKV→fMP4 fMP4 fragmentado para el browser).
- `recode_audio(path)` — solo si hace falta (AC3/TrueHD/DTS → AAC).
- **Servidor de bytes**: `GET /stream/:session` responde `Range` (o HLS) leyendo del
  `.part` en disco mientras baja.

### `store` — SQLite

- `/data/pistreaming.db`, modo WAL.
- Tablas: `addons`, `library`, `history`, `progress`, `metadata_cache`, `settings`.

### `api` — axum

- REST + endpoint de streaming + UI estática embebida (`rust-embed`).
- `tracing` para logs estructurados; errores JSON `{ error, code }`.

### `server` — binario

- Carga config (defaults < `/data/config.toml` < env).
- Levanta store, addon engine, torrent engine, media engine, router axum.
- Tareas tokio supervisadas con panic-recovery por tarea (un stream que truena no
  tumba el server).

---

## 5. Persistencia y estado

- **DB**: `/data/pistreaming.db` (SQLite, WAL).
- **Caché de torrents**: `/data/cache/<infohash>/` → `.part`/`.mkv` + `session.json`.
  Tope `CACHE_MAX_GB`, limpieza por antigüedad `CACHE_TTL_HOURS` (LRU), excluye `kept`.
- **Biblioteca**: `/data/library/` → `.mkv` fijado (hardlink desde caché si es el mismo
  filesystem, si no copia) + ficha en DB. `keep` promueve caché → biblioteca.
- **Progreso**: posición de reproducción por `(type,id)` para resume.
- **Precedencia de config**: defaults < `/data/config.toml` < variables de entorno.

---

## 6. Flujo de reproducción

```
GET  /api/search?query=...      → addon engine consulta catálogos de búsqueda
                                  (addons habilitados, en paralelo) → merge/dedupe
GET  /api/meta/:type/:id        → detalle (Cinemeta / addon de meta)
GET  /api/streams/:type/:id     → /stream de Torrentio → lista con infoHash/magnet
                                  + calidad/seeds/size, ranking 4K > 1080p > seeds>0
POST /api/play {stream}         → motor agrega magnet (con egress bind),
                                  activa streaming secuencial + últimas;
                                  ffprobe decide ruta → devuelve PlaybackPlan
GET  /stream/:session           → sirve bytes con Range (o HLS) desde el .part
                                  mientras baja (buffer en disco)
                                  al terminar: keep → biblioteca, si no se evapora del caché
GET/PUT /api/progress/:type/:id → resume
```

---

## 7. Regla de media fina (direct-play vs remux)

El video **NUNCA se transcodifica** (Pi sin encoder). Solo se cambia de contenedor:

- **Remux** = `ffmpeg -c copy` (MKV→fMP4), CPU ~0.
- Video **H.264** → remux; el browser lo reproduce.
- Video **HEVC/AV1** → remux igual, pero el browser puede no decodearlo → se devuelve
  además la **URL cruda** (`browser_may_fail=true`) para mpv/VLC. En el norte, la app
  oficial sí decodea HEVC.
- **Única recodificación aceptada**: audio AC3/TrueHD/DTS → AAC (barato en 4 cores).
- Decisión encapsulada en `media::decide()` con tabla de codecs → **unit-testeable**.

---

## 8. API surface (v1)

| Método | Ruta | Qué hace |
|---|---|---|
| GET | `/api/addons` | lista addons + estado |
| POST | `/api/addons` | agrega addon por URL (fetch manifest) |
| PATCH | `/api/addons/:id` | habilita/deshabilita/ordena |
| DELETE | `/api/addons/:id` | quita addon |
| GET | `/api/search?query=` | búsqueda federada |
| GET | `/api/meta/:type/:id` | detalle |
| GET | `/api/streams/:type/:id` | lista de streams rankeada |
| POST | `/api/play` | arranca sesión → `PlaybackPlan` |
| GET | `/stream/:session` | bytes/HLS desde el .part (Range) |
| GET | `/api/library` | biblioteca (`keep`) |
| GET/PUT | `/api/progress/:type/:id` | resume |
| GET | `/api/settings` · PUT | settings (egress, límites de caché) |
| GET | `/icon.svg` · `/` | icono + UI |

> **Refinado en Fase 2:** la clave de progreso es `{type}:{id}` sobre la tabla
> `progress` (PK `id`), no el `id` solo.

---

## 9. Frontend v1

- SPA mínima en **JS vanilla** (mismo approach que StreamVault: `index.html`, `app.js`,
  `style.css`), embebida en el binario con `rust-embed`. Sin build de npm.
- Vistas: **Addons** (agregar/activar/ordenar), **Buscar**, **Detalle** (ficha + streams
  con calidad/seeds y botón Play), **Reproductor** (`<video>` + fallback a URL cruda),
  **Biblioteca**.

---

## 10. Manejo de errores

- **Addon**: timeout + reintentos con backoff; aislamiento por addon; estado por addon
  (ok/degradado/caído) visible en la UI.
- **Torrent**: 0 seeds → stream marcado no reproducible; metadata no resuelta en 30 s →
  error claro. Sin panic.
- **Playback**: archivo sin piezas suficientes → esperar/retry; codec no soportado por
  el browser → exponer URL cruda.
- **API**: errores como `{ error, code }` con status HTTP correcto.
- **Proceso**: tareas tokio supervisadas, panic-recovery por tarea; el server sobrevive
  a fallos de una sesión.

---

## 11. Testing / verificación

- **Unit**: parseo de manifest; ranking de streams; `media::decide()` (tabla de codecs);
  limpieza de caché (LRU/TTL, respeta `kept`).
- **Integración**: addon mock (axum) que sirve manifest/catalog/stream → verifica que el
  engine consume, hace merge y rankea. Torrent de prueba chico para verificar streaming
  + Range servido desde `.part`.
- **E2E en el Pi**: reproducir un 1080p H.264 real en el navegador, de punta a punta
  (checkpoint manual).
- **Regla de done**: nada se da por terminado sin `cargo test` verde + un play real en el Pi.

---

## 12. Deploy — Docker + CasaOS

Mismo molde que StreamVault: single-container + app custom de CasaOS con bloque `x-casaos`.

- **Dockerfile multi-stage**: build `rust:1-slim-bookworm` → runtime `debian:bookworm-slim`
  + `ffmpeg` (solo para remux/recode de audio). `CARGO_BUILD_JOBS=2` para no ahogar el Pi.
- **Build arm64**: nativo en el Pi (recomendado). Alternativa: `buildx` arm64 (QEMU) — más lento.
- **Datos**: bind `/DATA/AppData/pistreaming` → `/data`.
- **Puerto**: `8092:8000` (8091 lo ocupa StreamVault).
- **Instalación**: `casaos-cli app-management install -f docker-compose.yml` (dry-run `-d` antes).
- **Acceso**: `http://192.168.100.100:8092` y Tailscale `http://100.126.97.89:8092`.

### Variables de entorno

| Var | Default | Qué hace |
|---|---|---|
| `TZ` | `America/La_Paz` | zona horaria |
| `EGRESS_BIND` | `eth0` | interfaz de salida del motor torrents (wg0 para VPN) |
| `CACHE_MAX_GB` | `40` | tope de la caché de torrents |
| `CACHE_TTL_HOURS` | `48` | antigüedad máxima antes de limpiar |
| `LIBRARY_DIR` | `/data/library` | directorio de `keep` |
| `DATA_DIR` | `/data` | base de DB/config/caché |
| `RUST_LOG` | `info` | nivel de logs |

---

## 13. Roadmap (norte, post-v1)

- Compatibilidad con el protocolo client-server de Stremio (apps oficiales apuntando al server).
- VPN con kill-switch (aprovechando `EGRESS_BIND` ya previsto).
- Subtítulos (addon `/subtitles`) y tracks de audio múltiples.
- Multi-usuario.

---

## 14. Riesgos

| Riesgo | Mitigación |
|---|---|
| Pi 4B compilando Rust pesado (librqbit) | `CARGO_BUILD_JOBS=2`, swap 5.9 GB, build nativo una sola vez |
| Browser no decodifica HEVC/AV1 | fallback a URL cruda (mpv/VLC); el norte usa app oficial |
| Addon externo caído/rate-limited | aislamiento por addon + timeout/backoff |
| Caché llena el disco | tope de tamaño + TTL LRU, excluye `kept` |
| Streaming ilegal / ISP | fuera de alcance del diseño (decisión del usuario); P0 sin VPN |
