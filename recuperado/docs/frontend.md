# Frontend (SPA) — versión desplegada

> `assets/app.js` (78.756 bytes), `assets/style.css` (26.183 bytes),
> `assets/index.html` (802 bytes), `assets/icon.svg` (171 bytes).
> Sin framework: DOM API, hash routing, sin `innerHTML`.

## Rutas

| Hash | Vista |
|---|---|
| `#/search` | Buscador (películas/series) |
| `#/detail/:kind/:id` | Ficha + fuentes |
| `#/player/:session` | Reproductor |
| `#/library` | Biblioteca |
| `#/addons` | Addons |
| `#/settings` | Ajustes |

## Funciones principales (extraídas de app.js)

- **Búsqueda**: `runSearch`, `searchHash`, `normalizeKind`, `viewSearch`,
  `metaCard`, `prefetchStreams`.
- **Ficha**: `viewDetail`, `detailHead`, `detailKey`, `renderSources`,
  `sourceRow`, `pickBest`, `pickEpisode`, `parseQuality`, `parseLang`,
  `parseSeeds`, `parseSize`, `parseProvider`, `isPack`.
- **Reproductor**: `startPlay`, `playerView`, `playerShell`, `viewPlayer`,
  `disposeActivePlayer`, `abortActive`, `recoverSession`, `replayBody`,
  `rememberReplay`, `knownReplay`, `knownSessionIdentity`,
  `identityFromProgressUrl`, `normalizeTracks`, `trackLabel`, `trackMeta`,
  `codecWarn`, `renderStreamError`, `renderSessionGone`.
- **Pre-warm**: `warmStream`, `warmBestSource`, `trackersOf`.
- **Progreso**: `fetch(progressUrl)`, `POST /api/progress/...`.
- **Biblioteca**: `viewLibrary`, `viewLibraryPlayer`, `libCard`.
- **Addons**: `viewAddons`, `openPanel`, `closePanel`, `syncPanel`.
- **Ajustes**: `viewSettings`, `setForceDirect`, `forceDirectEnabled`.
- **UI**: `el`, `svgIcon`, `showBanner`, `errorBox`, `backToDetail`,
  `clearFilters`, `roving`, `onDocClick`, `onDocKey`, `route`, `setTab`.

## Comportamiento destacado

- **Búsqueda con debounce** y caché de resultados en el cliente.
- **Prefetch de fuentes** al abrir la ficha; **pre-warm** del mejor torrent al
  asomar el puntero/tocar una fila (evita el 504 de fuente fría).
- **Reintento automático** en 504: re-POSTea `/api/play` una vez.
- **Sesión determinista**: si el servidor perdió la sesión (404), el cliente
  re-POSTea el mismo payload (`replayBody`) y recupera el plan.
- **Selección de fuente**: respeta el orden del servidor (x264 antes que
  HEVC/AV1, luego calidad/seeds); avisa de codecs que el navegador no reproduce.
- **Filtros de fuentes**: calidad, idioma, seeds, tamaño, proveedor.
- **Reproductor**: `<video>`, progreso, pista de audio (si el plan la expone),
  subtítulos embebidos y externos, `forceDirect` (forzar reproducción directa).
- **Subtítulos**: lista de pistas embebidas + búsqueda/descarga en
  OpenSubtitles (si `OPENSUBTITLES_API_KEY` está configurada) + subida externa.
- **Biblioteca**: guardar/reproducir/eliminar items.
- **Addons**: añadir por URL, activar/desactivar, eliminar.
- **Ajustes**: `cache_max_gb`, `cache_ttl_hours`, `egress_bind`, `http_port`,
  `data_dir`; avisa de los que requieren reinicio.
- **Sin XSS evidente**: no usa `innerHTML`; construye nodos con `el()` y
  `textContent`; `safePosterUrl` valida pósters.

## Estilos

`style.css` define el tema (topbar, tarjetas, panel de fuentes, reproductor,
badges, formularios, responsive). Paleta y variables CSS en `:root`.
