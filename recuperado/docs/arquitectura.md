# Arquitectura de PiStreaming (versión desplegada 2026-09-21)

> Reconstruida a partir de los símbolos, strings y comportamiento del binario
> `bin/pistreaming` (aarch64, no stripped, sin DWARF) y de la SPA recuperada.

## 1. Vista general

```
┌──────────────┐   HTTP    ┌─────────────────────────────────────────────┐
│  Navegador   │──────────▶│  pistreaming (Rust, axum, tokio)            │
│  SPA (app.js)│◀──────────│  :8000                                      │
└──────────────┘           │                                             │
                           │  ┌─────────┐  ┌──────────┐  ┌────────────┐  │
                           │  │ addons  │  │ torrent  │  │   media    │  │
                           │  │ (Stremio│  │ (librqbit│  │ (ffmpeg/   │  │
                           │  │  HTTP)  │  │  DHT)    │  │  ffprobe)  │  │
                           │  └────┬────┘  └────┬─────┘  └─────┬──────┘  │
                           │       │            │              │         │
                           │  ┌────▼────────────▼──────────────▼──────┐  │
                           │  │            store (SQLite WAL)          │  │
                           │  └────────────────────────────────────────┘  │
                           └─────────────────────────────────────────────┘
```

- **Runtime**: Tokio multi-thread, axum 0.7, tower-http.
- **Torrents**: `librqbit` 9.0.1 (DHT, magnet, selección de fichero, persistencia
  JSON en `cache/librqbit-state`).
- **Media**: `ffmpeg`/`ffprobe` del sistema (6.1.1 en el servidor).
- **Persistencia**: SQLite (rusqlite bundled) en `pistream-data/pistreaming.db`.
- **Assets**: embebidos con `rust-embed` y servidos por `/assets/*path`.

## 2. Workspace (7 crates)

| Crate | Responsabilidad | Ficheros vistos en el binario |
|---|---|---|
| `pistreaming-core` | Tipos y lógica pura: manifiestos, meta, streams, playback, URL | `crates/core/src/{catalog,error,lib,manifest,meta,playback,stream,url}.rs` |
| `pistreaming-store` | SQLite: addons, library, metadata_cache, play_sessions, progress, settings | `crates/store/src/lib.rs` |
| `pistreaming-addons` | Cliente de addons Stremio (manifest, search, meta, streams) | `crates/addons/src/lib.rs` |
| `pistreaming-torrent` | Motor librqbit: add magnet, elegir fichero, pre-warm, forget | `crates/torrent/src/{engine,session}.rs` |
| `pistreaming-media` | ffprobe (probe) y ffmpeg (remux/recode/subtítulos) | `crates/media/src/{decide,ffmpeg,lib,probe}.rs` |
| `pistreaming-api` | Router axum, handlers, streaming, caché de media, recuperación, OSDB | `crates/api/src/{assets,growing,library,lib,media_cache,osdb,range,recovery,session,settings}.rs` |
| `pistreaming-server` | `main.rs`: config, logging, arranque, señales | `crates/server/src/main.rs` |

## 3. Flujo de una reproducción

1. **Buscar** (`GET /api/search?query=&kind=`): consulta en paralelo los addons
   con recurso `catalog`; agrega y deduplica por `id`.
2. **Ficha** (`GET /api/meta/:kind/:id`): consulta addons con recurso `meta`.
3. **Fuentes** (`GET /api/streams/:kind/:id`): consulta addons con recurso
   `stream`; normaliza magnet/URL, rankea por reproducibilidad en navegador
   (`rank_streams`: x264 antes que HEVC/AV1, luego calidad/seeds).
4. **Pre-warm** (`POST /api/warm`): añade el magnet a librqbit y conecta peers
   para que el `play` no llegue frío.
5. **Play** (`POST /api/play`): resuelve metadata del torrent (timeout), elige
   fichero (`choose_file_id`/`pick_largest_video`), hace `ffprobe` (timeout 8 s)
   y decide plan (`decide_with_options`):
   - `direct`: el navegador reproduce el fichero tal cual.
   - `remux`: copia vídeo y audio a fMP4.
   - `recodeaudio`: copia vídeo, recodifica audio a AAC 192k.
   - `recode`: recodifica vídeo (HEVC/10-bit/Dolby Vision → H.264).
   Si el probe falla o expira: `fallback_plan` (Remux).
6. **Streaming** (`GET /stream/:session`): sirve `playback.mp4` mientras ffmpeg
   lo escribe (fichero creciente, `growing.rs`), con Range.
   `GET /raw/:session` sirve el fichero original del torrent.
7. **Progreso** (`GET/POST /api/progress/:kind/:id`): guarda posición/duración.
8. **Subtítulos**: embebidos (`/subtitles/:session/:track/raw`) y externos
   (`/api/subtitles/external/...`, OpenSubtitles vía `osdb.rs`).
9. **Cambio de audio** (`POST /api/play/:session/audio`): relanza ffmpeg con
   otra pista y re-persiste el plan.

## 4. Caché y limpieza

- `cache_max_gb` (40 GB) y `cache_ttl_hours` (4 h) en `settings`.
- `media_cache.rs`: transcode progresivo a `cache/<hash>/playback.mp4`,
  `transcode_done` en `play_sessions`, reintentos (`mark_retry`).
- `recovery.rs`: al arrancar restaura sesiones (`load_at_boot`), normaliza filas
  y reabre handles; cuarentena de `session.json` corrupto.
- Limpieza de caché stale y de `library` huérfana.
- `growing.rs`: sirve un fichero que aún se está escribiendo (Range, 416).

## 5. Base de datos (SQLite WAL)

```sql
addons(url PK, manifest_json, name, enabled, position, added_at)
library(id PK, kind, title, file_path, size_bytes, info_hash, created_at)
metadata_cache(key PK, json, expires_at)
play_sessions(id PK, info_hash, file_id, magnet, plan_json, kind, meta_id,
              title, progress_key, transcode_done, created_at)
progress(id PK, position, duration, updated_at)
settings(key PK, value)
```

- `play_sessions.id` es determinista: `<hash8>-<file_id>` (permite re-POSTear
  el mismo payload y recuperar la sesión).
- `transcode_done` marca el transcode completado.
- `metadata_cache` con TTL.

## 6. Configuración

| Variable | Default | Notas |
|---|---|---|
| `DATA_DIR` | `/data` | directorio de datos |
| `HTTP_PORT` | `8000` | puerto HTTP |
| `EGRESS_BIND` | `eth0` | interfaz para el tráfico torrent |
| `CACHE_MAX_GB` | `40` | límite de caché |
| `CACHE_TTL_HOURS` | `48` (default) / `4` (DB) | TTL de caché |
| `LIBRARY_DIR` | `<DATA_DIR>/library` | biblioteca |
| `OPENSUBTITLES_API_KEY` | — | subtítulos online |
| `PISTREAMING_CONFIG` | — | fichero de config alternativo |
| `RUST_LOG` | — | nivel de log |

Los ajustes `cache_max_gb`, `cache_ttl_hours`, `egress_bind`, `http_port` y
`data_dir` se persisten en la tabla `settings` y se editan desde la SPA
(`PUT /api/settings`). `egress_bind`, `http_port` y `data_dir` requieren
reinicio.

## 7. Pipeline ffmpeg (observado)

Remux/recodeaudio (fichero creciente):

```
ffmpeg -v error -y -re -probesize 1M -analyzeduration 0 -rw_timeout 15000000 \
  -i <raw_url> \
  -map 0:v:0 -map 0:a:0? -c:v copy -c:a aac -b:a 192k -ac 2 \
  -flush_packets 1 -movflags frag_keyframe+empty_moov+default_base_moof \
  -f mp4 <cache>/<hash>/playback.mp4
```

- `-re` limita la lectura a tiempo real (no descarga el torrent entero de golpe).
- Recode de vídeo: `-c:v libx264 -preset veryfast -crf … -maxrate … -bufsize …`.
- Subtítulos a VTT: `ffmpeg -v error -y -i <input> -map 0:s:0 -f webvtt -`.

## 8. Recuperación de sesiones

- `recovery::load_at_boot`: lee `play_sessions`, reabre torrents y handles.
- `ensure_session` / `ensure_handle`: recrean si faltan.
- `normalize_row`: sanea filas antiguas.
- `quarantine_corrupt_session_file`: aparta `session.json` corrupto de
  librqbit.
- El frontend tiene `recoverSession`/`replayBody` para re-POSTear el plan si el
  servidor perdió la sesión (404).

## 9. Frontend (SPA)

- Hash routing: `#/search`, `#/detail/:kind/:id`, `#/player/:session`,
  `#/library`, `#/addons`, `#/settings`.
- Sin framework: DOM API (`el()`), sin `innerHTML` (evita XSS).
- Búsqueda con debounce, caché de resultados y prefetch de fuentes.
- Reproductor: `<video>`, progreso, pista de audio, subtítulos, `forceDirect`.
- Pre-warm al asomar el puntero/tocar una fila.
- Reintento automático en 504 (fuente fría).
- Ver `docs/frontend.md` para el detalle.
