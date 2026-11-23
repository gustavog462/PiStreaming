# Estructura del fuente (reconstruida del binario)

> El binario no tiene DWARF, pero conserva `.symtab` y las rutas de fichero de
> los eventos `tracing`. Con eso se reconstruye la estructura de módulos,
> funciones y tamaños aproximados.

## Árbol reconstruido

```
PiStreaming/
├── Cargo.toml                  # workspace (7 crates)
├── Cargo.lock
├── rust-toolchain.toml
├── crates/
│   ├── core/
│   │   └── src/
│   │       ├── catalog.rs
│   │       ├── error.rs
│   │       ├── lib.rs
│   │       ├── manifest.rs
│   │       ├── meta.rs
│   │       ├── playback.rs
│   │       ├── stream.rs       # rank_streams
│   │       └── url.rs          # normalize_url
│   ├── store/
│   │   └── src/lib.rs          # link_or_copy, play_session_from_row, validate_name_segment
│   ├── addons/
│   │   └── src/lib.rs          # AddonManager, manifest/search/meta/streams
│   ├── torrent/
│   │   └── src/
│   │       ├── engine.rs       # add_magnet_*, choose_file_id, forget, pick_largest_video…
│   │       └── session.rs
│   ├── media/
│   │   └── src/
│   │       ├── decide.rs       # audio_needs_recode, decide_with_options
│   │       ├── ffmpeg.rs       # ffmpeg_to_file, remux/recode, subtitle_vtt_command
│   │       ├── lib.rs
│   │       └── probe.rs        # build_cmd, collect_tracks, probe_from_json, run_with_timeout
│   ├── api/
│   │   ├── assets/             # app.js, style.css, index.html, icon.svg (rust-embed)
│   │   └── src/
│   │       ├── assets.rs       # serve
│   │       ├── growing.rs      # fichero creciente (Range)
│   │       ├── library.rs      # mime_for_path
│   │       ├── lib.rs          # router + handlers (play, warm, stream, subtitles…)
│   │       ├── media_cache.rs  # transcode progresivo + reintentos
│   │       ├── osdb.rs         # OpenSubtitles
│   │       ├── range.rs        # parse_range, ranged_response
│   │       ├── recovery.rs     # load_at_boot, ensure_session/handle
│   │       ├── session.rs      # AsyncReadSeek
│   │       └── settings.rs     # validación de ajustes
│   └── server/
│       └── src/main.rs         # config, logging, arranque
└── docs/
```

## Tamaños aproximados (líneas, del binario)

| Fichero | Líneas (máx. vista) |
|---|---|
| `crates/api/src/lib.rs` | 1.571 |
| `crates/server/src/main.rs` | 416 |
| `crates/torrent/src/engine.rs` | 335 |
| `crates/addons/src/lib.rs` | 325 |
| `crates/media/src/ffmpeg.rs` | 309 |
| `crates/api/src/media_cache.rs` | 286 |
| `crates/api/src/recovery.rs` | 184 |

(Los demás ficheros no tienen eventos `tracing` con línea, así que no se puede
estimar su tamaño; el repo `fase2-playback` da una idea de los que ya existían.)

## Funciones propias (86, sin wrappers)

### `pistreaming_api`
- `router`, `play`, `warm`, `switch_audio`, `subtitles`, `put_external_subtitle`,
  `vtt_response`, `stream`, `apply_plan_base`, `fallback_plan`,
  `base_from_headers`, `encode_magnet_value`, `normalize_query`, `no_space`,
  `core_err`, `err`, `spawn_search_refresh`.
- `assets::serve`, `library::mime_for_path`, `settings::field_err`,
  `osdb::client`.
- `growing::{complete_response, file_len, range_not_satisfiable, range_start}`.
- `range::{num, parse_range, ranged_response}`.
- `media_cache::{ensure_started, mark_retry, poll_done, started, started_err}`.
- `recovery::{ensure_handle, ensure_session, load_at_boot, normalize_row, unix_now}`.

### `pistreaming_torrent`
- `engine::{add_magnet_peered, add_magnet_resilient_inner, add_opts,
  choose_file_id, forget, is_video_name, pick_largest_video,
  quarantine_corrupt_session_file, session_opts}`.

### `pistreaming_media`
- `ffmpeg::{drain_stderr, ffmpeg_to_file, input_args, output_args,
  recode_audio_to_file, recode_to_file, remux_to_file, subtitle_vtt_command}`.
- `probe::{build_cmd, collect_tracks, probe_from_json, probe_once, run_with_timeout}`.
- `decide::{audio_needs_recode, decide_with_options}`.

### `pistreaming_store`
- `link_or_copy`, `play_session_from_row`, `validate_name_segment`.

### `pistreaming_core`
- `stream::rank_streams`, `url::normalize_url`.

## Diferencias con el repo `fase2-playback` (2026-09-14)

Ficheros nuevos en el binario (no están en el repo):

- `crates/api/src/growing.rs`
- `crates/api/src/media_cache.rs`
- `crates/api/src/osdb.rs`
- `crates/api/src/recovery.rs`
- `crates/torrent/src/session.rs`

Rutas nuevas en el binario (no están en el repo):

- `POST /api/warm`
- `POST /api/play/:session/audio`
- `GET /api/play/:session/subtitles/:track/raw`
- `GET /api/subtitles/external/:kind/:id`
- `POST /api/subtitles/external/:kind/:id`
- `GET /api/subtitles/external/:kind/:id/file/:lang`
- `GET /api/library/:id/stream`
- `GET /api/addons/:url/stream/:session`

## Cómo regenerar este análisis

```bash
nm -C bin/pistreaming | grep -E "pistreaming_" | sort -u > docs/simbolos.txt
strings bin/pistreaming | grep -oE "crates/[a-z_]+/src/[a-zA-Z0-9_/]+\.rs" | sort -u
strings bin/pistreaming | grep -oE "crates/[a-z_]+/src/[a-zA-Z0-9_/]+\.rs:[0-9]+" | sort -u
```

Ver `tools/analizar-binario.sh`.
