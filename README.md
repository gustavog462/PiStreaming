# PiStreaming

Servidor de streaming personal tipo Stremio para Raspberry Pi, escrito en Rust.

## Estado

- **Código fuente**: rama `fase2-playback` (fase 2, 2026-09-14) y
  `fase0-fase1` (reconstrucción del núcleo + HLS, 2026-11-23).
- **Recuperación del despliegue** (2026-09-21): carpeta [`recuperado/`](recuperado/README.md).
- El árbol de fuente original se perdió; `recuperado/` contiene el binario, los
  assets de la SPA, la configuración, la base de datos y la documentación
  reconstruida a partir del binario desplegado.

## Estructura

- `crates/` — workspace Rust: `core`, `store`, `addons`, `api`, `server`,
  `torrent`, `media`.
- `docs/superpowers/` — planes y specs de las fases 1–3.
- `recuperado/` — material recuperado del servidor (assets, configs, docs,
  datos).
- `docs/fase0-fase1.md` — qué se implementó en la reconstrucción (growing,
  media_cache, recovery, osdb, HLS, pistas de audio/subtítulos).
- `docs/fase3.md` — autenticación (contraseña + cookie, API key, token de
  embed) e i18n ES/EN.

## Documentación de la recuperación

- [`recuperado/README.md`](recuperado/README.md)
- [`recuperado/RECUPERACION.md`](recuperado/RECUPERACION.md)
- [`recuperado/docs/arquitectura.md`](recuperado/docs/arquitectura.md)
- [`recuperado/docs/api.md`](recuperado/docs/api.md)
- [`recuperado/docs/estructura-fuente.md`](recuperado/docs/estructura-fuente.md)
