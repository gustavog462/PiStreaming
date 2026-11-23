# Recuperación de PiStreaming — 2026-11-22

> Informe de qué se extrajo del servidor, cómo, y qué se perdió.

## Contexto

El proyecto original se estaba desarrollando en el PC. Al parecer se perdió el
árbol de código fuente. El servidor (`piserver`) conserva el **binario
desplegado** y sus **datos**, pero no el fuente. El repo de GitHub
(`gustavog462/PiStreaming`) existe pero se quedó en la fase2 (2026-09-14).

## Qué se ha recuperado

### 1. Binario desplegado

- Ruta original: `/home/pi/pistreaming` (servidor)
- Copia local: `bin/pistreaming`
- Tipo: ELF 64-bit LSB pie executable, ARM aarch64, dinámico
- Tamaño: 24.664.664 bytes
- BuildID: `3792b56c905c97b9ee27c583e59d0619761673f1`
- SHA-256: `d21764a23e82bcb965d6506125a68c939e45fdfd6ac6761eaa8d8b91cc0f41f5`
- Compilado: 2026-09-21 (mtime del fichero)
- No stripped: conserva `.symtab` (1.365 símbolos `pistreaming_*`)
- Sin DWARF (no hay `.debug_info`), así que no se puede recuperar el código
  fuente, pero sí nombres de módulos, funciones y rutas de fichero.

### 2. Assets de la SPA

Descargados del servidor en ejecución (`http://<host>:8000`):

| Fichero | Bytes | Origen |
|---|---|---|
| `assets/app.js` | 78.756 | `/assets/app.js` |
| `assets/style.css` | 26.183 | `/assets/style.css` |
| `assets/index.html` | 802 | `/` |
| `assets/icon.svg` | 171 | `/icon.svg` |

Son la versión desplegada (fase3+), no la del repo (16.545 bytes de app.js).

### 3. Configuración

- `config/pistreaming.service` — unit systemd (`/etc/systemd/system/pistreaming.service`)
- `config/logrotate-pistreaming` — `/etc/logrotate.d/pistreaming`
- `config/env.example` — variables de entorno detectadas en el binario

### 4. Datos

- `data/pistreaming.db` — copia consistente vía API `sqlite3.backup` (5.308.416 bytes)
- `data/schema.sql` — esquema de las 6 tablas
- `data/addons.json` — 9 addons con manifiestos
- `data/settings.json` — ajustes efectivos
- `data/counts.json` — conteo de tablas
- `data/manifest.txt` — manifiesto del directorio de datos

### 5. Log

- `logs/pistream.log` — 1,4 MB, desde la instalación hasta 2026-11-22

### 6. Análisis del binario

- `docs/simbolos.txt` — 1.365 símbolos `pistreaming_*` (demangled)
- `docs/funciones.txt` — 86 funciones propias (sin wrappers de `alloc`/`core`)
- `docs/estructura-fuente.txt` — rutas de fichero `.rs` embebidas
- `docs/eventos-lineas.txt` — 58 eventos `tracing` con fichero:línea
- `docs/rutas.txt` — rutas HTTP detectadas
- `docs/strings-relevantes.txt` — mensajes de error, env vars y args de ffmpeg
- `docs/sistema.txt` — info del servidor, hashes y versiones

## Qué NO se ha podido recuperar

- **El código fuente Rust** de los 7 crates. No está en el PC (búsqueda en todo
  el disco), ni en el servidor, ni en el repo (el repo tiene la versión del
  14-sep, anterior a la desplegada).
- El directorio de compilación `/home/pi/pistreaming-src` (referenciado en los
  símbolos del binario) ya no existe.
- No hay DWARF en el binario, así que no hay tablas de líneas ni tipos que
  permitan reconstruir el fuente automáticamente.

## Reconstrucción de la estructura

A partir de los símbolos y strings del binario se ha reconstruido la estructura
de módulos y funciones (ver `docs/estructura-fuente.md`). Sirve como guía para
reescribir el fuente o para comparar con la rama `fase2-playback` del repo.

## Diferencias repo (2026-09-14) vs binario (2026-09-21)

| Elemento | Repo `fase2-playback` | Binario desplegado |
|---|---|---|
| `crates/api/src/lib.rs` | 1.213 líneas | ~1.571 líneas |
| `assets/app.js` | 16.545 bytes | 78.756 bytes |
| `crates/api/src/osdb.rs` | ❌ | ✅ |
| `crates/api/src/media_cache.rs` | ❌ | ✅ |
| `crates/api/src/growing.rs` | ❌ | ✅ |
| `crates/api/src/recovery.rs` | ❌ | ✅ |
| `crates/torrent/src/session.rs` | ❌ | ✅ |
| `/api/warm` | ❌ | ✅ |
| `/api/play/:session/audio` | ❌ | ✅ |
| `/api/subtitles/external/...` | ❌ | ✅ |
| `/api/library/:id/stream` | ❌ | ✅ |
| `/api/addons/:url/stream/:session` | ❌ | ✅ |

## Cómo se hizo (reproducible)

Ver `tools/extraer.sh`, `tools/dump-db.py` y `tools/analizar-binario.sh`.
Resumen:

1. SSH al servidor (`pi@<host>`).
2. `curl` de los assets al puerto 8000.
3. `scp` del binario, configs y log.
4. `python3` + `sqlite3.backup` para la DB; export de esquema/addons/ajustes.
5. `nm -C`, `strings` y `readelf` para el análisis del binario.
6. Limpieza de artefactos de prueba (torrent de Sintel) y reinicio del servicio.

## Nota de seguridad

El token de GitHub usado para actualizar el repo aparece en logs de sesiones de
agentes en el PC. **Rótalo** en GitHub → Settings → Developer settings →
Personal access tokens. Caduca el 2026-12-21.
