# API de PiStreaming (versión desplegada 2026-09-21)

> Reconstruida a partir de la SPA (`assets/app.js`), de las rutas embebidas en
> el binario y de pruebas contra el servidor. Base: `http://<host>:8000`.

## Convenciones

- Respuestas JSON salvo streaming/subtítulos.
- Errores: `{"code": <http>, "error": "<mensaje>"}`.
- Sin autenticación (ver `docs/seguridad.md`).
- `kind` ∈ `movie` | `series`.
- `id` es el id del addon (p. ej. IMDb `tt0133093`; series `tt…:T:E`).

## Salud

### `GET /api/health`
```json
{"status":"ok"}
```

## Addons

### `GET /api/addons`
Lista de addons con su manifiesto.
```json
[{"url":"https://v3-cinemeta.strem.io","manifest":{...}}]
```

### `POST /api/addons`
Añade un addon. Body: `{"url":"https://…"}`.
Normaliza la URL (quita `/manifest.json`).

### `PATCH /api/addons/:url`
Activa/desactiva. Body: `{"enabled": true|false}`.

### `DELETE /api/addons/:url`
Elimina el addon.

### `GET /api/addons/:url/stream/:session`
Proxy/stream asociado a un addon (usado por fuentes no-torrent).

## Catálogo y metadatos

### `GET /api/search?query=<q>&kind=<kind>`
```json
{"metas":[{"id":"tt0133093","name":"The Matrix","poster":"…","type":"movie"}]}
```
Consulta en paralelo los addons con recurso `catalog`; deduplica por `id`.

### `GET /api/meta/:kind/:id`
Detalle (título, sinopsis, póster, fondo, géneros, duración, reparto, vídeos).
Consulta addons con recurso `meta`.

### `GET /api/streams/:kind/:id`
```json
{"streams":[{"name":"Torrentio\n1080p","title":"…","infoHash":"…","fileIdx":5,
             "url":null,"sources":[],"source_addon":"Torrentio"}]}
```
Normaliza magnet/URL y rankea por reproducibilidad en navegador
(`rank_streams`): x264 antes que HEVC/AV1, luego calidad/seeds.

## Reproducción

### `POST /api/play`
Body:
```json
{"magnet":"magnet:?xt=urn:btih:…","title":"…","kind":"movie","id":"tt…",
 "fileIdx":5,"forceDirect":false}
```
Respuesta (plan):
```json
{"session":"08ada5a7a618-5","route":"recodeaudio",
 "playback_url":"http://<host>:8000/stream/08ada5a7a618-5",
 "raw_url":"http://<host>:8000/raw/08ada5a7a618-5?file_id=5",
 "browser_may_fail":true,"needs_recode_audio":true,
 "video_codec":"","audio_codec":null,"audio_tracks":[],
 "subtitle_tracks":[],"active_audio_index":null,
 "progress_url":"http://<host>:8000/api/progress/movie/tt…"}
```
- `route` ∈ `direct` | `remux` | `recodeaudio` | `recode`.
- `session` = `<hash8>-<file_id>` (determinista).
- Errores: 504 si no resuelve metadata a tiempo; 507 si disco lleno;
  500 si el torrent no se puede decodificar.

### `GET /api/play/:session`
Devuelve el plan guardado (404 si la sesión no existe).

### `POST /api/play/:session/audio`
Cambia la pista de audio. Body: `{"index": 1}`. Relanza ffmpeg y re-persiste el
plan. Error si la fuente no lo permite.

### `GET /api/play/:session/subtitles/:track/raw`
Subtítulo embebido en crudo.

### `POST /api/warm`
Pre-calienta un torrent. Body:
```json
{"info_hash":"…","trackers":["udp://…"],"fileIdx":5}
```
Idempotente; conecta peers para que el `play` no llegue frío.

### `GET /stream/:session`
Streaming del `playback.mp4` creciente (Range, 206/416).

### `GET /raw/:session?file_id=<n>`
Fichero original del torrent (Range).

### `GET /subtitles/:session/:track/raw`
Subtítulo embebido (VTT).

## Progreso

### `GET /api/progress/:kind/:id`
```json
{"position":123.4,"duration":7200.0}
```

### `POST /api/progress/:kind/:id`
Body: `{"position":123.4,"duration":7200.0}`.

## Ajustes

### `GET /api/settings`
```json
{"cache_max_gb":40,"cache_ttl_hours":4,"data_dir":"…","egress_bind":"eth0",
 "http_port":8000,"requires_restart":["egress_bind","http_port","data_dir"]}
```

### `PUT /api/settings`
Body con los campos a cambiar. Devuelve `{"applied":[…],"requires_restart":[…]}`.
Valida `egress_bind` (interfaz existente), `http_port` (rango) y rutas.

## Biblioteca

### `GET /api/library`
Lista de items guardados.

### `POST /api/library`
Añade un item (kind, title, file_path, size_bytes, info_hash).

### `DELETE /api/library/:id`
Elimina el item.

### `GET /api/library/:id/stream`
Sirve el fichero local de la biblioteca (Range, MIME por extensión).

## Subtítulos externos (OpenSubtitles)

### `GET /api/subtitles/external/:kind/:id`
Lista candidatos (requiere `OPENSUBTITLES_API_KEY`).

### `POST /api/subtitles/external/:kind/:id`
Sube un subtítulo externo (VTT/SRT).

### `GET /api/subtitles/external/:kind/:id/file/:lang`
Descarga el subtítulo en VTT.

## Assets y SPA

- `GET /assets/*path` — estáticos embebidos (`app.js`, `style.css`, …).
- `GET /icon.svg` — icono.
- `GET /play/:session` — ruta SPA del reproductor.
- `GET /` — `index.html` de la SPA.

## Códigos de error observados

| Código | Caso |
|---|---|
| 400 | body inválido / JSON mal formado |
| 404 | recurso o sesión no encontrada |
| 405 | método no permitido |
| 500 | error interno (torrent no decodificable, addon caído) |
| 504 | metadata del torrent no resuelta a tiempo (fuente fría) |
| 507 | disco lleno en `/api/play` |
