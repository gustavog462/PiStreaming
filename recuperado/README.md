# PiStreaming — proyecto recuperado

> Servidor de streaming personal tipo Stremio para Raspberry Pi.
> Esta carpeta es la **recuperación** del proyecto desplegado en el servidor
> (`piserver`), hecha el **2026-11-22**, después de perder el árbol de código
> fuente original en el PC.

## Qué es

PiStreaming es una aplicación **Rust** (workspace de 7 crates) que:

- Se conecta a **addons de Stremio** (Cinemeta, Torrentio, Comet, MediaFusion,
  TMDB, Anime Kitsu, TorrentsDB) para catálogos, metadatos y fuentes.
- Reproduce **torrents en streaming** con [`librqbit`](https://github.com/ikatson/rqbit)
  (DHT, magnet, selección de fichero, pre-warm de peers).
- Sirve el vídeo por HTTP con **Range** y, cuando hace falta, **remux/transcode
  con ffmpeg** (copia de vídeo + AAC, o recode completo) a un fichero
  `playback.mp4` que se sirve mientras crece.
- Tiene una **SPA** en español (buscar, ficha, fuentes, reproductor,
  biblioteca, addons, ajustes) con progreso, cambio de pista de audio y
  subtítulos (embebidos y OpenSubtitles).
- Persiste addons, sesiones de reproducción, progreso, biblioteca y ajustes en
  **SQLite** (WAL).

## Estado de la recuperación

| Elemento | Estado |
|---|---|
| Binario desplegado (`bin/pistreaming`) | ✅ recuperado (aarch64, 24 MB, no stripped) |
| Assets de la SPA (`assets/`) | ✅ recuperados del servidor en ejecución |
| Configuración (`config/`) | ✅ systemd + logrotate |
| Base de datos (`data/pistreaming.db`) | ✅ copia consistente (solo tabla `addons` con datos) |
| Esquema, addons y ajustes exportados (`data/`) | ✅ |
| Log (`logs/pistream.log`) | ✅ (local; contiene datos de uso) |
| **Código fuente Rust** | ❌ **perdido** (no está en el PC ni en el servidor) |
| Repo GitHub | ✅ existe `gustavog462/PiStreaming`, pero **desactualizado** (2026-09-14) |

El binario desplegado es del **2026-09-21**; el repo de GitHub se quedó en
**2026-09-14**. La diferencia (~1 semana) incluye la SPA completa, subtítulos,
pre-warm, cambio de audio, caché de media y recuperación de sesiones.

## Estructura de esta carpeta

```
PiStreaming/
├── README.md              ← este fichero
├── RECUPERACION.md        ← informe detallado de la recuperación
├── assets/                ← SPA servida por el binario (app.js, style.css, index.html, icon.svg)
├── bin/pistreaming        ← binario desplegado (aarch64)
├── config/                ← pistreaming.service, logrotate, env.example
├── data/                  ← DB, esquema, addons, ajustes, manifiesto
├── docs/                  ← arquitectura, API, frontend, despliegue, seguridad, estructura
├── logs/pistream.log      ← log del servicio
└── tools/                 ← scripts de extracción y análisis
```

## Cómo se ejecuta (servidor)

```bash
# servicio systemd
sudo systemctl status pistreaming
sudo systemctl restart pistreaming

# datos
DATA_DIR=/home/pi/pistream-data
# HTTP
http://<host>:8000/
```

Variables de entorno (ver `config/env.example`):

```
DATA_DIR, HTTP_PORT, EGRESS_BIND, CACHE_MAX_GB, CACHE_TTL_HOURS,
LIBRARY_DIR, OPENSUBTITLES_API_KEY, RUST_LOG
```

## Qué falta para reconstruir el fuente

1. **Código Rust** de los 7 crates (ver `docs/estructura-fuente.md` para la
   estructura reconstruida a partir de símbolos y strings del binario).
2. Los **assets** ya están recuperados y son la versión desplegada; se pueden
   reincorporar a `crates/api/assets/`.
3. El **esquema SQLite** está en `data/schema.sql`.
4. La **API** está documentada en `docs/api.md` (rutas, métodos y
   comportamiento observados).
5. El repo de GitHub tiene la versión del 14-sep (fase2) que sirve como base
   para continuar.

## Aviso

El repo de GitHub es **público**. En esta carpeta local hay datos que **no**
deben subirse a un repo público: `logs/`, `data/pistreaming.db` (rutas y datos
de uso) y el binario (contiene rutas de compilación con el usuario). Para el
repo se publica una versión saneada (assets, configs genéricas, docs, esquema
y addons).
