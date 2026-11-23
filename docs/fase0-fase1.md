# Fase 0 y Fase 1 — implementación

> Reconstrucción del núcleo Rust de PiStreaming (fase 0) y módulo HLS (fase 1)
> sobre la base de `fase2-playback` + el material recuperado del binario.

## Fase 0 — paridad con el binario desplegado

### Módulos nuevos

| Módulo | Qué hace |
|---|---|
| `crates/api/src/growing.rs` | Sirve un fichero que aún se escribe (Range, espera a que crezca, 416). |
| `crates/api/src/media_cache.rs` | Transcode progresivo a `cache/<hash>/playback.mp4` con ffmpeg (`-c:v copy`, audio AAC), `transcode_done`, parada/reset. |
| `crates/api/src/recovery.rs` | Persiste y recupera sesiones (`play_sessions`), `ensure_session`/`ensure_handle` bajo demanda, poda. |
| `crates/api/src/osdb.rs` | Cliente mínimo de OpenSubtitles (idiomas, descarga, SRT→VTT). |
| `crates/api/src/hls.rs` | HLS con ffmpeg (fase 1). |

### Cambios en lo existente

- **`core/playback.rs`**: `TrackInfo` y campos nuevos en `PlaybackPlan`:
  `audio_tracks`, `subtitle_tracks`, `active_audio_index`.
- **`media/probe.rs`**: parsea pistas de audio/subtítulos (índice, codec, idioma,
  título, default, forced).
- **`media/decide.rs`**: copia las pistas al plan y marca la pista de audio
  activa (la `default` o la primera).
- **`store`**: tabla `play_sessions` + `save/list/get/delete/set_transcode_done/prune`.
- **`api/lib.rs`**:
  - `POST /api/play` persiste la sesión y devuelve pistas.
  - `GET /stream/:session` para Remux/RecodeAudio usa `media_cache` + `growing`
    (fichero creciente, seekable) en vez de stdout de un solo consumidor.
  - `GET /raw/:session`, `GET /api/play/:session` y `GET /stream/:session`
    recrean la sesión desde la DB si hace falta.
  - `POST /api/warm` (pre-calienta torrent).
  - `POST /api/play/:session/audio` (cambio de pista + reinicio del transcode).
  - `GET /api/play/:session/subtitles/:track/raw` y `/subtitles/:session/:track/raw`
    (subtítulos embebidos a VTT con ffmpeg).
  - `GET/PUT/POST /api/subtitles/external/:kind/:id` y
    `GET /api/subtitles/external/:kind/:id/file/:lang` (OpenSubtitles).
  - `forceDirect` en `POST /api/play`.
- **`server/main.rs`**: `recovery::load_at_boot` al arrancar, poda de sesiones,
  borrado de la fila persistida al cerrar una sesión stale, parada del HLS.
- **Assets**: se sustituyen por la SPA recuperada del binario desplegado
  (`app.js` 78 KB, `style.css` 26 KB, `index.html`, `icon.svg`).

### Tests

- `cargo test --workspace`: **todos pasan** (48 en `pistreaming-api` + resto).
- Nuevos: `crates/api/tests/phase0.rs` (pistas en el plan, cambio de audio,
  subtítulos sin API key, warm sin engine, HLS sobre direct).
- Tests unitarios de `growing`, `media_cache`, `recovery`, `osdb`, `hls`.

## Fase 1 — HLS con ffmpeg

### Endpoints

- `GET /hls/:session/index.m3u8` — playlist HLS (event, crece).
- `GET /hls/:session/seg_00000.m4s` — segmentos fMP4.

### Cómo funciona

```
ffmpeg -v error -y -re -i <raw_url> \
  -map 0:v:0 -map 0:a:0? -c:v copy -c:a aac -b:a 192k -ac 2 \
  -f hls -hls_time 4 -hls_playlist_type event \
  -hls_segment_type fmp4 -hls_flags independent_segments \
  -hls_segment_filename <cache>/<hash>/hls/seg_%05d.m4s \
  <cache>/<hash>/hls/index.m3u8
```

- **Vídeo siempre `-c:v copy`**: el Pi 5 no tiene encoder hardware y el recode
  de vídeo en tiempo real no es viable.
- Audio a AAC solo cuando hace falta (`RecodeAudio`); en `Remux` se copia.
- Sin `nginx-vod` (AGPL) ni `content-transcoder` (Go): solo ffmpeg + axum.
- El playlist es `event`, así que el player puede empezar con los primeros
  segmentos y seguir creciendo.
- El ffmpeg HLS se guarda en un mapa estático por sesión; `hls::stop` lo mata
  al cambiar de audio o al cerrar la sesión.

### Uso

- Reproductor externo: `http://<host>:8000/hls/<session>/index.m3u8`.
- La SPA actual (recuperada) sigue usando `/stream/:session` (MP4 progresivo);
  integrar HLS en la SPA con `hls.js` queda para la fase 2.

## Cómo compilar y probar

```bash
# local (x86_64)
cargo build --release
cargo test --workspace

# Pi (aarch64)
rsync -az --exclude target --exclude .git ./ gustavo@<pi>:~/pistreaming-src/
ssh gustavo@<pi> 'cd ~/pistreaming-src && source ~/.cargo/env && CARGO_BUILD_JOBS=2 cargo build --release'
```

## Pendiente (fase 2+)

- Integrar HLS en la SPA (hls.js) y elegir HLS vs MP4 según el cliente.
- Persistir `transcode_done` desde la tarea de media_cache (hoy se actualiza al
  servir `/stream`).
- Tests end-to-end con un torrent real en el Pi (medir CPU/RAM).
- ZIP al vuelo, miniaturas, traducción de subtítulos, auth, i18n.
- Endurecer systemd (`MemoryMax`, `ProtectSystem`, etc.).
