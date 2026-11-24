# Fase 4 — Discover, Continuar viendo, métricas y CI

## Discover

- Pestaña **Descubrir** (`#/discover`) con:
  - **Continuar viendo**: progreso en curso (empezado y no terminado) desde la
    tabla `progress`, con barra de progreso y enlace a la ficha.
  - **Películas populares** y **Series populares**: catálogos `top` de los
    addons (Cinemeta).
- API:
  - `GET /api/catalog/:kind/:id` → `{ "metas": [...] }` (catálogo de los addons
    que lo exponen, en paralelo con timeout global).
  - `GET /api/continue` → `{ "items": [{kind,id,position,duration,updated_at}] }`.
- La SPA enriquece cada item de «Continuar viendo» con `/api/meta` (en paralelo)
  y pinta la barra de progreso.

## Métricas Prometheus

- `GET /metrics` (público) en formato texto:
  - `pistreaming_requests_total{method,path,status}` (middleware, ruta por
    patrón con `MatchedPath`).
  - `pistreaming_play_sessions`, `pistreaming_torrents`,
    `pistreaming_ffmpeg`, `pistreaming_cache_bytes` (tamaño **asignado** en
    disco, no aparente), `pistreaming_uptime_seconds`,
    `pistreaming_build_info`.
- El gauge de caché se actualiza cada 60 s con un walk de metadata.

## CI

- `.github/workflows/ci.yml`: `cargo fmt --check`, `cargo clippy -D warnings`,
  `cargo test --workspace`, `cargo build --release` en cada push/PR.

## Tests

- `crates/api/tests/phase4.rs`: continuar viendo (filtra terminados), catálogo
  sin addons, `/metrics`, y auth (401 sin credencial, API key, login+cookie).
- `cargo test --workspace` en verde; `cargo clippy` limpio.

## Pendiente

- Integraciones WebDAV/S3/RealDebrid/TorBox (fase 4 opcional restante).
- Traducción de subtítulos con IA (descartada).
