# PiStreaming — Fase 3 (SPA embebida + endpoints faltantes) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use omo-subagent-driven-development (recommended) or omo-dispatching-parallel-agents to implement this plan task-by-task. Each task should specify a `category` (quick/deep/ultrabrain/visual-engineering) and `load_skills` for oh-my-opencode's task() tool. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Completar el producto de punta a punta: una SPA vanilla embebida en el binario (`GET /`) con las vistas Buscar/Detalle/Reproductor/Biblioteca/Addons/Ajustes, más la biblioteca permanente (`keep` a `/data/library`) y los endpoints que le faltan (`GET|POST|DELETE /api/library`, `GET /library/:id/stream`, `PATCH /api/addons/:url`, `GET|PUT /api/settings`).

**Architecture:** Los assets HTML/CSS/JS viven en `crates/api/assets/` y se embeben con `rust-embed` en un módulo nuevo `crates/api/src/assets.rs` (en debug lee del disco, en release embebe los bytes). La SPA navega por hash (`#/search`, `#/detail/:kind/:id`, `#/player/:session`, `#/library`, `#/addons`, `#/settings`), así que el servidor no necesita fallback de rutas: todo entra por `/`. La biblioteca es una tabla nueva `library` en `pistreaming-store` con su CRUD; `keep` promueve el `.mkv`/`.mp4` cacheado de una sesión a `<data_dir>/library` con hardlink (fallback a copia si el FS destino difiere) y la limpieza LRU/TTL excluye por `info_hash` lo guardado. Los ajustes que aplican en caliente (`cache_max_gb`, `cache_ttl_hours`) viven en `RuntimeSettings` con `AtomicU64` dentro de `AppState`, y el evictor los lee en cada tick; los que requieren reinicio (`egress_bind`, `http_port`, `data_dir`) se persisten en la tabla `settings` y se marcan como tales.

**Tech Stack:** Rust 2021, `axum` 0.7 (se mantiene), `rust-embed` 8, `rusqlite` (bundled), `tokio` (`fs`, `sync::OnceCell`, `sync::Mutex`, `sync::RwLock`), `parking_lot`, `serde`/`serde_json`, `librqbit` 9.0.1 (`ManagedTorrent::stats().finished`, `ManagedTorrent::output_folder()`), SPA HTML/CSS/JS plana (sin npm).

---

## Ajustes al spec detectados en el anclaje (decisiones que el plan sí implementa)

El spec `docs/superpowers/specs/2026-09-13-pistreaming-fase3-spa-design.md` se implementa tal cual, con estas precisiones verificadas contra el código real:

1. **`PATCH /api/addons/:url` (no `:id`).** El spec §5.2 dice `:id`, pero el router ya direcciona addons por su URL (`DELETE /api/addons/:url`, `crates/api/src/lib.rs:55`) y `Store::set_addon_enabled(url, enabled)` ya existe (`crates/store/src/lib.rs:100`). El identificador es la URL, normalizada con `normalize_url` y decodificada con `decode_url`.
2. **`Store::keep` recibe campos explícitos.** El spec §4.4 escribe `keep(session) -> LibraryItem`, pero `pistreaming-store` no depende de `pistreaming-api` (donde vive `PlaySession`). El store recibe `(library_dir, kind, id, title, info_hash, src)`; el handler resuelve la `PlaySession` y arma la ficha.
3. **`AppState.library_dir` explícito.** El spec §4.1 dice que `library_dir` deriva de `data_dir`, pero `AppState.cache_dir` **no** siempre es `<data_dir>/cache` (en los tests es el tempdir raíz). Se agrega `library_dir: PathBuf` a `AppState`; `main` lo fija a `<data_dir>/library` y `test_state` a `<tmp>/library`.
4. **Completitud de descarga real.** El spec §4.2 dice «la descarga no está completa → 409». Se usa la API real de librqbit 9.0.1: `ManagedTorrent::stats().finished` (`torrent_state/mod.rs:503`). Si no hay handle vivo para la sesión, también es 409.
5. **`data_dir` es bootstrap-only.** El spec §5.3 marca `data_dir` como «requiere reinicio»; como la DB SQLite vive dentro de `data_dir`, no puede recargarse sin reabrir el store. `main` advierte si hay un `data_dir` persistido distinto del activo; `egress_bind` y `http_port` **sí** se aplican al reiniciar leyendo lo persistido. `cache_max_gb`/`cache_ttl_hours` también se leen al arrancar.
6. **La biblioteca no vive bajo `cache_dir`.** `<data_dir>/library` está fuera de `cache_dir`, así que la limpieza por nombre de carpeta nunca lo ve; además se excluye por `info_hash` de la tabla `library` (spec §4.3/§8).
7. **`/play/:session` se conserva como redirect a la SPA.** El spec manda eliminar `player.html` y su handler; para no romper enlaces viejos, `GET /play/:session` responde `307` a `/#/player/:session`.
8. **`link_or_copy` intenta hardlink y cae a copia ante cualquier error.** Comportamiento observable del spec §7 («Hardlink entre dispositivos distintos → fallback a copia»); evita depender de `ErrorKind::CrossesDevices` y cubre también permisos/FS no soportado.

---

## File Structure

**Crear**
- `crates/api/src/settings.rs` — `RuntimeSettings` (`AtomicU64`) + `StaticSettings` + handlers `get_settings`/`put_settings` + tests inline.
- `crates/api/src/assets.rs` — `#[derive(RustEmbed)] struct Assets` + `index`/`asset`/`icon`/`play_redirect` + tests inline.
- `crates/api/src/library.rs` — handlers de biblioteca (`list`/`keep`/`delete`/`stream`).
- `crates/api/assets/index.html` — shell de la SPA (barra superior + `#app`).
- `crates/api/assets/style.css` — tema «cine oscuro — negro + rosa».
- `crates/api/assets/app.js` — router por hash + vistas.
- `crates/api/assets/icon.svg` — favicon.
- `crates/api/tests/library_api.rs` — API de biblioteca (`GET|POST|DELETE`, `/library/:id/stream`).
- `crates/api/tests/spa_assets.rs` — `/` y `/assets/*` sirven la SPA con su `Content-Type`.

**Modificar**
- `Cargo.toml` — dep de workspace `rust-embed`.
- `crates/api/Cargo.toml` — `rust-embed.workspace = true`.
- `crates/store/src/lib.rs` — tabla `library` en `MIGRATIONS`, `LibraryItem`, `link_or_copy`, CRUD y `library_info_hashes`.
- `crates/api/src/lib.rs` — `pub mod settings; pub mod assets; pub mod library;`, `AppState` extendido, router (rutas nuevas, se quita `player`), `err`/`core_err`/`decode_url` a `pub(crate)`, `test_state` extendido.
- `crates/api/src/session.rs` — `PlaySession` con `kind`/`id`/`title`/`media_path`.
- `crates/server/src/main.rs` — carga settings persistidos, `AppState` extendido, evictor lee settings al vuelo y excluye `library`.
- `crates/api/tests/play_progress.rs` — constructor de `PlaySession` con los campos nuevos.
- `crates/api/tests/player.rs` — `/play/:session` ahora redirige a la SPA.

**Eliminar**
- `crates/api/assets/player.html` — reemplazado por la SPA.

---

## Task 1: `store` — tabla `library` + CRUD + `link_or_copy`

Categoría sugerida: `quick`. Sin skills extra.

**Files:**
- Modify: `crates/store/src/lib.rs` (const `MIGRATIONS` líneas 11-35; nuevo `LibraryItem` tras `AddonRow` línea 44; `impl Store` tras `remove_addon` línea 118; tests al final del módulo `tests` línea 283)

- [ ] **Step 1: Escribir los tests que fallan**

Al final de `crates/store/src/lib.rs`, dentro del `mod tests` existente (después del test `metadata_cache_hits_then_expires`, línea 282), agregar:

```rust
    #[test]
    fn link_or_copy_enlaza_en_mismo_fs() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.bin");
        let dst = dir.path().join("dst.bin");
        std::fs::write(&src, b"hola").unwrap();

        link_or_copy(&src, &dst).unwrap();

        assert_eq!(std::fs::read(&dst).unwrap(), b"hola");
        // Mismo inodo en el mismo FS => se tomó la rama hardlink (no copia).
        let mi = std::fs::metadata(&src).unwrap();
        let md = std::fs::metadata(&dst).unwrap();
        assert_eq!(mi.ino(), md.ino(), "debe ser hardlink en el mismo FS");
    }

    #[test]
    fn copy_file_copia_el_contenido() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.bin");
        let dst = dir.path().join("dst.bin");
        std::fs::write(&src, b"datos").unwrap();

        copy_file(&src, &dst).unwrap();

        assert_eq!(std::fs::read(&dst).unwrap(), b"datos");
    }

    #[test]
    fn library_keep_list_remove() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();
        let library_dir = dir.path().join("library");
        let src = dir.path().join("cache").join("movie.mkv");
        std::fs::create_dir_all(src.parent().unwrap()).unwrap();
        std::fs::write(&src, vec![7u8; 1024]).unwrap();

        let item = store
            .keep(&library_dir, "movie", "bbb-2014", "Beauty", Some("hash1"), &src)
            .unwrap();

        assert_eq!(item.id, "movie:bbb-2014");
        assert_eq!(item.kind, "movie");
        assert_eq!(item.title, "Beauty");
        assert_eq!(item.size_bytes, 1024);
        assert_eq!(item.info_hash.as_deref(), Some("hash1"));
        assert!(
            std::path::Path::new(&item.file_path).exists(),
            "el archivo debe existir en la biblioteca"
        );
        assert_eq!(store.list_library().unwrap().len(), 1);

        store.remove_library("movie:bbb-2014").unwrap();
        assert!(store.list_library().unwrap().is_empty());
        assert!(
            !std::path::Path::new(&item.file_path).exists(),
            "remove_library borra el archivo"
        );
    }

    #[test]
    fn library_keep_es_idempotente_sobre_el_mismo_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();
        let library_dir = dir.path().join("library");
        let src = dir.path().join("a.mkv");
        std::fs::write(&src, b"uno").unwrap();

        store.keep(&library_dir, "movie", "x", "X", None, &src).unwrap();
        let again = store.keep(&library_dir, "movie", "x", "X", None, &src).unwrap();

        assert_eq!(store.list_library().unwrap().len(), 1, "sin duplicar la fila");
        assert_eq!(again.id, "movie:x");
    }

    #[test]
    fn remove_library_es_idempotente_con_archivo_faltante() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();
        let library_dir = dir.path().join("library");
        let src = dir.path().join("m.mkv");
        std::fs::write(&src, b"x").unwrap();

        let item = store.keep(&library_dir, "movie", "z", "Z", None, &src).unwrap();
        std::fs::remove_file(&item.file_path).unwrap();

        // No debe fallar aunque el archivo ya no esté.
        store.remove_library("movie:z").unwrap();
        assert!(store.list_library().unwrap().is_empty());
        // Borrar una fila que no existe tampoco falla.
        store.remove_library("movie:no-existe").unwrap();
    }

    #[test]
    fn library_info_hashes_lista_solo_los_guardados() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();
        let library_dir = dir.path().join("library");
        let a = dir.path().join("a.mkv");
        let b = dir.path().join("b.mkv");
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();

        store.keep(&library_dir, "movie", "a", "A", Some("hashA"), &a).unwrap();
        store.keep(&library_dir, "movie", "b", "B", None, &b).unwrap();

        assert_eq!(store.library_info_hashes().unwrap(), vec!["hashA".to_string()]);
    }
```

- [ ] **Step 2: Correr y ver el fallo**

Run: `cd ~/Proyectos/PiStreaming && cargo test -p pistreaming-store library_`

Expected: FAIL de compilación: `cannot find type LibraryItem`, `cannot find function link_or_copy` / `copy_file`, y `no method named keep/list_library/remove_library/library_info_hashes`.

- [ ] **Step 3: Implementación mínima**

**3a)** Reemplazar la const `MIGRATIONS` (líneas 11-35) por (se agrega la tabla `library` al final del batch):

```rust
const MIGRATIONS: &str = r#"
CREATE TABLE IF NOT EXISTS addons (
  url           TEXT PRIMARY KEY,
  manifest_json TEXT NOT NULL,
  name          TEXT NOT NULL,
  enabled       INTEGER NOT NULL DEFAULT 1,
  position      INTEGER NOT NULL DEFAULT 0,
  added_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE IF NOT EXISTS settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS progress (
  id         TEXT PRIMARY KEY,
  position   REAL NOT NULL DEFAULT 0,
  duration   REAL,
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE IF NOT EXISTS metadata_cache (
  key        TEXT PRIMARY KEY,
  json       TEXT NOT NULL,
  expires_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS library (
  id          TEXT PRIMARY KEY,
  kind        TEXT NOT NULL,
  title       TEXT NOT NULL,
  file_path   TEXT NOT NULL,
  size_bytes  INTEGER NOT NULL,
  info_hash   TEXT,
  created_at  INTEGER NOT NULL
);
"#;
```

**3b)** Añadir, tras los imports actuales (líneas 3-9), el `LibraryItem` y los helpers de enlace/copia:

```rust
/// Una entrada de la biblioteca permanente (spec §4.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LibraryItem {
    /// `"<kind>:<id>"`, p. ej. `"movie:bbb-2014"`.
    pub id: String,
    /// `movie` | `series`.
    pub kind: String,
    pub title: String,
    /// Ruta absoluta del archivo guardado en `library_dir`.
    pub file_path: String,
    pub size_bytes: i64,
    /// `info_hash` del torrent origen, para que la limpieza no toque lo guardado.
    pub info_hash: Option<String>,
    /// Segundos Unix.
    pub created_at: i64,
}

/// Enlaza `src` a `dst` con hardlink; si el FS destino difiere (o el enlace falla),
/// copia el contenido. Cubre `EXDEV` y cualquier FS sin hardlinks.
pub fn link_or_copy(src: &Path, dst: &Path) -> CoreResult<()> {
    if std::fs::hard_link(src, dst).is_ok() {
        return Ok(());
    }
    copy_file(src, dst)
}

/// Copia `src` -> `dst` (fallback de `link_or_copy`).
fn copy_file(src: &Path, dst: &Path) -> CoreResult<()> {
    std::fs::copy(src, dst)
        .map(|_| ())
        .map_err(|e| CoreError::Other(format!(
            "no se pudo copiar {} -> {}: {e}",
            src.display(),
            dst.display()
        )))
}
```

**3c)** Dentro de `impl Store`, después de `remove_addon` (línea 118), agregar:

```rust
    /// Promueve el archivo `src` de una sesión a `library_dir` y registra la ficha.
    ///
    /// El nombre del archivo es `"<kind>:<id>.<ext>"` (el `:` es válido en Linux).
    /// Es idempotente sobre el mismo `id`: reemplaza la copia y actualiza la fila.
    pub fn keep(
        &self,
        library_dir: &Path,
        kind: &str,
        id: &str,
        title: &str,
        info_hash: Option<&str>,
        src: &Path,
    ) -> CoreResult<LibraryItem> {
        if !src.exists() {
            return Err(CoreError::NotFound(format!(
                "archivo de origen no existe: {}",
                src.display()
            )));
        }
        let full_id = format!("{kind}:{id}");
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .filter(|e| !e.is_empty())
            .unwrap_or("mkv");
        std::fs::create_dir_all(library_dir).map_err(|e| {
            CoreError::Other(format!("no se pudo crear {}: {e}", library_dir.display()))
        })?;
        let dst = library_dir.join(format!("{full_id}.{ext}"));
        if dst.exists() {
            std::fs::remove_file(&dst).map_err(|e| {
                CoreError::Other(format!("no se pudo reemplazar {}: {e}", dst.display()))
            })?;
        }
        link_or_copy(src, &dst)?;
        let size_bytes = std::fs::metadata(&dst)
            .map(|m| m.len() as i64)
            .map_err(|e| CoreError::Other(format!("no se pudo medir {}: {e}", dst.display())))?;
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let item = LibraryItem {
            id: full_id,
            kind: kind.to_string(),
            title: title.to_string(),
            file_path: dst.to_string_lossy().into_owned(),
            size_bytes,
            info_hash: info_hash.map(str::to_string),
            created_at,
        };
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO library (id, kind, title, file_path, size_bytes, info_hash, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET kind = excluded.kind,
                                           title = excluded.title,
                                           file_path = excluded.file_path,
                                           size_bytes = excluded.size_bytes,
                                           info_hash = excluded.info_hash",
            params![
                item.id,
                item.kind,
                item.title,
                item.file_path,
                item.size_bytes,
                item.info_hash,
                item.created_at
            ],
        )
        .map_err(|e| CoreError::Db(e.to_string()))?;
        Ok(item)
    }

    /// Lista la biblioteca, más reciente primero.
    pub fn list_library(&self) -> CoreResult<Vec<LibraryItem>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, kind, title, file_path, size_bytes, info_hash, created_at
                 FROM library ORDER BY created_at DESC, id",
            )
            .map_err(|e| CoreError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(LibraryItem {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    title: row.get(2)?,
                    file_path: row.get(3)?,
                    size_bytes: row.get(4)?,
                    info_hash: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })
            .map_err(|e| CoreError::Db(e.to_string()))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| CoreError::Db(e.to_string()))?);
        }
        Ok(out)
    }

    /// Borra el archivo (si existe) y la fila. Idempotente.
    pub fn remove_library(&self, id: &str) -> CoreResult<()> {
        let file_path: Option<String> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT file_path FROM library WHERE id = ?1")
                .map_err(|e| CoreError::Db(e.to_string()))?;
            let mut rows = stmt.query(params![id]).map_err(|e| CoreError::Db(e.to_string()))?;
            match rows.next().map_err(|e| CoreError::Db(e.to_string()))? {
                Some(row) => Some(row.get(0).map_err(|e| CoreError::Db(e.to_string()))?),
                None => None,
            }
        };
        if let Some(p) = &file_path {
            match std::fs::remove_file(p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(CoreError::Other(format!("no se pudo borrar {p}: {e}")))
                }
            }
        }
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM library WHERE id = ?1", params![id])
            .map_err(|e| CoreError::Db(e.to_string()))?;
        Ok(())
    }

    /// `info_hash` de lo guardado, para que la limpieza LRU/TTL lo excluya.
    pub fn library_info_hashes(&self) -> CoreResult<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT info_hash FROM library WHERE info_hash IS NOT NULL AND info_hash <> ''")
            .map_err(|e| CoreError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| CoreError::Db(e.to_string()))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| CoreError::Db(e.to_string()))?);
        }
        Ok(out)
    }
```

- [ ] **Step 4: Correr y ver pasar**

Run: `cd ~/Proyectos/PiStreaming && cargo test -p pistreaming-store`

Expected: PASS (los 6 tests previos + los 6 nuevos).

- [ ] **Step 5: Commit**

```bash
git add crates/store/src/lib.rs
git commit -m "feat(store): tabla library, CRUD, link_or_copy y hashes para la limpieza"
```

---
## Task 2: `api` — `settings.rs` (`RuntimeSettings` + `StaticSettings` + GET/PUT) y `AppState` extendido

Categoría sugerida: `deep`. Skills: ninguna (lógica Rust + axum).

**Files:**
- Create: `crates/api/src/settings.rs`
- Modify: `crates/api/src/lib.rs` (imports 1-23; `pub mod` 22-23; `AppState` 25-47; `fn err` 71; `fn core_err` 78; `router` 51-69; `test_state` 569-584; literal de test 631-642; literal inline 708-718)
- Modify: `crates/server/src/main.rs` (líneas 95-128 y usos de `cfg.egress_bind`/`cfg.http_port` en 111-112 y 147)

- [ ] **Step 1: Escribir el test que falla**

Crear `crates/api/src/settings.rs` con este contenido completo (los tests no compilan todavía porque `AppState` no tiene `settings`/`static_settings`):

```rust
//! Ajustes de la app: los que aplican en caliente (atomics) y los que exigen reinicio.

use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::Value;

use crate::SharedState;

/// Claves persistidas en la tabla `settings`.
pub const KEY_CACHE_MAX_GB: &str = "cache_max_gb";
pub const KEY_CACHE_TTL_HOURS: &str = "cache_ttl_hours";
pub const KEY_EGRESS_BIND: &str = "egress_bind";
pub const KEY_HTTP_PORT: &str = "http_port";
pub const KEY_DATA_DIR: &str = "data_dir";

/// Ajustes que el evictor lee en cada tick (aplican sin reiniciar).
pub struct RuntimeSettings {
    cache_max_gb: AtomicU64,
    cache_ttl_hours: AtomicU64,
}

impl RuntimeSettings {
    pub fn new(cache_max_gb: u64, cache_ttl_hours: u64) -> Self {
        Self {
            cache_max_gb: AtomicU64::new(cache_max_gb),
            cache_ttl_hours: AtomicU64::new(cache_ttl_hours),
        }
    }

    pub fn cache_max_gb(&self) -> u64 {
        self.cache_max_gb.load(Ordering::Relaxed)
    }

    pub fn cache_ttl_hours(&self) -> u64 {
        self.cache_ttl_hours.load(Ordering::Relaxed)
    }

    pub fn set_cache_max_gb(&self, v: u64) {
        self.cache_max_gb.store(v, Ordering::Relaxed);
    }

    pub fn set_cache_ttl_hours(&self, v: u64) {
        self.cache_ttl_hours.store(v, Ordering::Relaxed);
    }
}

/// Valores activos de arranque para los campos que solo aplican reiniciando.
#[derive(Clone)]
pub struct StaticSettings {
    pub egress_bind: String,
    pub http_port: u16,
    pub data_dir: std::path::PathBuf,
}

/// GET /api/settings — valores efectivos + qué campo exige reinicio.
pub async fn get_settings(State(st): State<SharedState>) -> Response {
    let persisted = |key: &str| st.store.get_setting(key).ok().flatten();
    let egress_bind = persisted(KEY_EGRESS_BIND)
        .unwrap_or_else(|| st.static_settings.egress_bind.clone());
    let data_dir = persisted(KEY_DATA_DIR)
        .unwrap_or_else(|| st.static_settings.data_dir.to_string_lossy().into_owned());
    let http_port = persisted(KEY_HTTP_PORT)
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(st.static_settings.http_port);
    Json(serde_json::json!({
        "cache_max_gb": st.settings.cache_max_gb(),
        "cache_ttl_hours": st.settings.cache_ttl_hours(),
        "egress_bind": egress_bind,
        "data_dir": data_dir,
        "http_port": http_port,
        "requires_restart": [KEY_EGRESS_BIND, KEY_HTTP_PORT, KEY_DATA_DIR],
    }))
    .into_response()
}

fn field_err(field: &str) -> Response {
    crate::err(
        StatusCode::BAD_REQUEST,
        format!("valor inválido en `{field}`"),
    )
}

/// PUT /api/settings — aplica `cache_max_gb`/`cache_ttl_hours` en caliente;
/// persiste `egress_bind`/`http_port`/`data_dir` y los marca como reinicio.
pub async fn put_settings(
    State(st): State<SharedState>,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    let Json(v) = match body {
        Ok(b) => b,
        Err(_) => return crate::err(StatusCode::BAD_REQUEST, "body inválido"),
    };
    let Some(obj) = v.as_object() else {
        return crate::err(StatusCode::BAD_REQUEST, "body inválido");
    };

    let mut applied: Vec<&str> = Vec::new();
    let mut requires_restart: Vec<&str> = Vec::new();

    if let Some(val) = obj.get(KEY_CACHE_MAX_GB) {
        let Some(n) = val.as_u64().filter(|n| *n > 0) else {
            return field_err(KEY_CACHE_MAX_GB);
        };
        st.settings.set_cache_max_gb(n);
        if let Err(e) = st.store.set_setting(KEY_CACHE_MAX_GB, &n.to_string()) {
            return crate::core_err(e);
        }
        applied.push(KEY_CACHE_MAX_GB);
    }
    if let Some(val) = obj.get(KEY_CACHE_TTL_HOURS) {
        let Some(n) = val.as_u64().filter(|n| *n > 0) else {
            return field_err(KEY_CACHE_TTL_HOURS);
        };
        st.settings.set_cache_ttl_hours(n);
        if let Err(e) = st.store.set_setting(KEY_CACHE_TTL_HOURS, &n.to_string()) {
            return crate::core_err(e);
        }
        applied.push(KEY_CACHE_TTL_HOURS);
    }
    if let Some(val) = obj.get(KEY_EGRESS_BIND) {
        let Some(s) = val.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
            return field_err(KEY_EGRESS_BIND);
        };
        if let Err(e) = st.store.set_setting(KEY_EGRESS_BIND, s) {
            return crate::core_err(e);
        }
        requires_restart.push(KEY_EGRESS_BIND);
    }
    if let Some(val) = obj.get(KEY_HTTP_PORT) {
        let Some(n) = val.as_u64().filter(|n| (1..=65535).contains(n)) else {
            return field_err(KEY_HTTP_PORT);
        };
        if let Err(e) = st.store.set_setting(KEY_HTTP_PORT, &n.to_string()) {
            return crate::core_err(e);
        }
        requires_restart.push(KEY_HTTP_PORT);
    }
    if let Some(val) = obj.get(KEY_DATA_DIR) {
        let Some(s) = val.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
            return field_err(KEY_DATA_DIR);
        };
        if let Err(e) = st.store.set_setting(KEY_DATA_DIR, s) {
            return crate::core_err(e);
        }
        requires_restart.push(KEY_DATA_DIR);
    }

    Json(serde_json::json!({ "applied": applied, "requires_restart": requires_restart }))
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn app_con(tmp: &tempfile::TempDir) -> (SharedState, axum::Router) {
        let st = crate::test_state(tmp.path().to_path_buf());
        let app = crate::router(st.clone());
        (st, app)
    }

    #[tokio::test]
    async fn put_aplica_caliente_y_marca_reinicio() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, app) = app_con(&tmp);

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/settings")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"cache_max_gb":12,"http_port":9090}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["applied"], serde_json::json!(["cache_max_gb"]));
        assert_eq!(v["requires_restart"], serde_json::json!(["http_port"]));

        // En caliente: el runtime ya lo refleja sin reiniciar.
        assert_eq!(st.settings.cache_max_gb(), 12);
        assert_eq!(st.settings.cache_ttl_hours(), 48);

        // GET refleja lo persistido y lista los campos de reinicio.
        let res = app
            .oneshot(Request::get("/api/settings").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["cache_max_gb"], 12);
        assert_eq!(v["http_port"], 9090);
        assert_eq!(
            v["requires_restart"],
            serde_json::json!(["egress_bind", "http_port", "data_dir"])
        );
    }

    #[tokio::test]
    async fn put_valor_invalido_es_400_con_el_campo() {
        let tmp = tempfile::tempdir().unwrap();
        let (_st, app) = app_con(&tmp);

        let res = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/settings")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"cache_max_gb":"mucho"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            v["error"].as_str().unwrap().contains("cache_max_gb"),
            "el 400 debe indicar el campo: {v}"
        );
    }

    #[test]
    fn runtime_settings_set_y_get() {
        let s = RuntimeSettings::new(10, 24);
        assert_eq!(s.cache_max_gb(), 10);
        assert_eq!(s.cache_ttl_hours(), 24);
        s.set_cache_max_gb(99);
        s.set_cache_ttl_hours(3);
        assert_eq!(s.cache_max_gb(), 99);
        assert_eq!(s.cache_ttl_hours(), 3);
    }
}
```

- [ ] **Step 2: Correr y ver el fallo**

Run: `cd ~/Proyectos/PiStreaming && cargo test -p pistreaming-api settings::`

Expected: FAIL de compilación: `unresolved module settings` (no declarado), `no field settings/static_settings on type AppState`, `function 'err'/'core_err' is private`.

- [ ] **Step 3: Implementación mínima**

**3a)** En `crates/api/src/lib.rs`, cambiar los módulos (líneas 22-23) de:

```rust
pub mod range;
pub mod session;
```

a:

```rust
pub mod assets;
pub mod library;
pub mod range;
pub mod session;
pub mod settings;
```

**3b)** Extender `AppState` (bloque 25-47) agregando tres campos al final (antes de la llave de cierre):

```rust
    /// Ajustes que aplican en caliente; el evictor los lee en cada tick.
    pub settings: StdArc<settings::RuntimeSettings>,
    /// Valores activos de arranque para los campos que exigen reinicio.
    pub static_settings: settings::StaticSettings,
    /// Directorio de la biblioteca permanente (`<data_dir>/library`).
    pub library_dir: std::path::PathBuf,
```

**3c)** Hacer visibles helpers internos: en la línea 71 reemplazar

```rust
fn err(code: StatusCode, msg: impl Into<String>) -> Response {
```

por

```rust
pub(crate) fn err(code: StatusCode, msg: impl Into<String>) -> Response {
```

y en la línea 78 reemplazar

```rust
fn core_err(e: CoreError) -> Response {
```

por

```rust
pub(crate) fn core_err(e: CoreError) -> Response {
```

**3d)** En `router` (líneas 51-69), agregar antes de `.with_state(state)`:

```rust
        .route(
            "/api/settings",
            get(settings::get_settings).put(settings::put_settings),
        )
```

**3e)** Reemplazar el cuerpo de `test_state` (líneas 569-584) por:

```rust
pub fn test_state(dir: std::path::PathBuf) -> SharedState {
    let store = Store::open(&dir.join("pistreaming.db")).expect("store");
    let client = AddonClient::new(reqwest::Client::new());
    let addons = AddonManager::new(client.clone());
    let library_dir = dir.join("library");
    StdArc::new(AppState {
        store,
        client,
        addons: StdArc::new(RwLock::new(addons)),
        mutex: Mutex::new(()),
        torrents: tokio::sync::OnceCell::new(),
        cache_dir: dir.clone(),
        public_base: "http://127.0.0.1:8000".to_string(),
        sessions: PlaySessionRegistry::new(),
        handles: parking_lot::RwLock::new(std::collections::HashMap::new()),
        settings: StdArc::new(settings::RuntimeSettings::new(40, 48)),
        static_settings: settings::StaticSettings {
            egress_bind: "eth0".to_string(),
            http_port: 8000,
            data_dir: dir,
        },
        library_dir,
    })
}
```

**3f)** En el `mod tests` de `lib.rs`, actualizar el helper `test_state()` (líneas 631-642) y el literal inline (líneas 708-718) agregando los tres campos, con `dir` disponible:

```rust
            settings: Arc::new(settings::RuntimeSettings::new(40, 48)),
            static_settings: settings::StaticSettings {
                egress_bind: "eth0".to_string(),
                http_port: 8000,
                data_dir: dir.path().to_path_buf(),
            },
            library_dir: dir.path().join("library"),
```

(insertar esos tres campos inmediatamente después de `handles: parking_lot::RwLock::new(std::collections::HashMap::new()),` en ambos literales).

**3g)** En `crates/server/src/main.rs`, agregar el alias de claves dentro de `main()` (tras la línea 95, `let cfg_path = ...`):

```rust
    use pistreaming_api::settings as settings_keys;
```

**3h)** Reemplazar el bloque `let state = Arc::new(AppState { ... });` (líneas 118-128) por la carga de settings persistidos + el literal extendido:

```rust
    let cache_max_gb = store
        .get_setting(settings_keys::KEY_CACHE_MAX_GB)
        .ok()
        .flatten()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(cfg.cache_max_gb);
    let cache_ttl_hours = store
        .get_setting(settings_keys::KEY_CACHE_TTL_HOURS)
        .ok()
        .flatten()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(cfg.cache_ttl_hours);
    let egress_bind = store
        .get_setting(settings_keys::KEY_EGRESS_BIND)
        .ok()
        .flatten()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| cfg.egress_bind.clone());
    let http_port = store
        .get_setting(settings_keys::KEY_HTTP_PORT)
        .ok()
        .flatten()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(cfg.http_port);
    if let Ok(Some(persisted)) = store.get_setting(settings_keys::KEY_DATA_DIR) {
        if persisted != cfg.data_dir.to_string_lossy() {
            tracing::warn!(
                persisted = %persisted,
                activo = %cfg.data_dir.display(),
                "data_dir persistido no aplica en caliente; editá la config y reiniciá"
            );
        }
    }

    let state = Arc::new(AppState {
        store,
        client,
        addons: Arc::new(RwLock::new(mgr)),
        mutex: Mutex::new(()),
        torrents: torrents_cell,
        cache_dir: cache_dir.clone(),
        public_base: format!("http://127.0.0.1:{http_port}"),
        sessions: registry,
        handles: parking_lot::RwLock::new(Default::default()),
        settings: Arc::new(pistreaming_api::settings::RuntimeSettings::new(
            cache_max_gb,
            cache_ttl_hours,
        )),
        static_settings: pistreaming_api::settings::StaticSettings {
            egress_bind: egress_bind.clone(),
            http_port,
            data_dir: cfg.data_dir.clone(),
        },
        library_dir: cfg.data_dir.join("library"),
    });
```

**3i)** Reemplazar las líneas 111-112 y 147 para usar los valores efectivos: `cfg.egress_bind` -> `egress_bind` y `cfg.http_port` -> `http_port`:

```rust
    let torrent_session = open_session(cache_dir.clone(), Some(egress_bind.clone())).await?;
    tracing::info!(bind = %egress_bind, "sesión torrent abierta");
```

```rust
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], http_port));
```

> `store.get_setting` se llama antes de mover `store` al `AppState`; es `&self`, así que no lo consume.

- [ ] **Step 4: Correr y ver pasar**

Run: `cd ~/Proyectos/PiStreaming && cargo test -p pistreaming-api settings:: && cargo build -p pistreaming-server`

Expected: PASS (3 tests de `settings`) y compila el binario.

- [ ] **Step 5: Commit**

```bash
git add crates/api/src/settings.rs crates/api/src/lib.rs crates/server/src/main.rs
git commit -m "feat(api): settings en caliente con RuntimeSettings y GET|PUT /api/settings"
```

---
## Task 3: `server` — el evictor lee settings al vuelo y excluye la biblioteca

Categoría sugerida: `quick`.

**Files:**
- Modify: `crates/server/src/main.rs` (spawn del evictor 130-143; `evict_cache` 154-247; tests 290-351)
- Test: tests inline en `crates/server/src/main.rs`

- [ ] **Step 1: Escribir el test que falla**

Dentro del `mod tests` de `crates/server/src/main.rs` (después del test `evict_cache_no_borra_sesion_activa`, línea 351), agregar:

```rust
    /// La limpieza no debe evictar la carpeta de caché de un torrent ya guardado
    /// en la biblioteca (se excluye por `info_hash`).
    #[tokio::test]
    async fn evict_cache_no_toca_lo_guardado_en_biblioteca() {
        use super::evict_cache;

        let dir = tempfile::tempdir().unwrap();
        let state = pistreaming_api::test_state(dir.path().to_path_buf());

        let kept_hash = "cccccccccccccccccccccccccccccccccccccccc";
        let cache_dir = dir.path().join(kept_hash);
        std::fs::create_dir_all(&cache_dir).unwrap();
        let media = cache_dir.join("movie.mkv");
        std::fs::write(&media, vec![0u8; 512]).unwrap();

        state
            .store
            .keep(&state.library_dir, "movie", "z", "Z", Some(kept_hash), &media)
            .unwrap();

        // cache_max_gb = 0 fuerza evicción por tamaño; TTL alto evita cerrar sesiones.
        state.settings.set_cache_max_gb(0);
        evict_cache(&state).await.unwrap();

        assert!(
            cache_dir.exists(),
            "la caché de algo guardado en la biblioteca no debe evictarse"
        );
    }

    /// El evictor lee el TTL en caliente: con TTL 0 evicta una carpeta recién usada.
    #[tokio::test]
    async fn evict_cache_lee_el_ttl_en_caliente() {
        use super::evict_cache;

        let dir = tempfile::tempdir().unwrap();
        let state = pistreaming_api::test_state(dir.path().to_path_buf());

        let hash = "dddddddddddddddddddddddddddddddddddddddd";
        let cache_dir = dir.path().join(hash);
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::write(cache_dir.join("f.mkv"), vec![0u8; 64]).unwrap();

        state.settings.set_cache_max_gb(0);
        state.settings.set_cache_ttl_hours(0);
        evict_cache(&state).await.unwrap();

        assert!(!cache_dir.exists(), "TTL 0 debe evictar la carpeta");
    }
```

- [ ] **Step 2: Correr y ver el fallo**

Run: `cd ~/Proyectos/PiStreaming && cargo test -p pistreaming-server evict_cache`

Expected: FAIL de compilación: `this function takes 3 arguments but 1 argument was supplied` (el `evict_cache` actual recibe `(state, cache_max_gb, cache_ttl_hours)`).

- [ ] **Step 3: Implementación mínima**

**3a)** Reemplazar el spawn del evictor (líneas 130-143) por:

```rust
    // Job de eviction: cada 10 min, cierra sesiones inactivas fuera de TTL y
    // evicta por tamaño/TTL. Lee los ajustes en caliente y nunca toca la biblioteca.
    let evict_state = state.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(600));
        loop {
            tick.tick().await;
            if let Err(e) = evict_cache(&evict_state).await {
                tracing::warn!(error = %e, "eviction falló");
            }
        }
    });
```

**3b)** Reemplazar la firma y el inicio de `evict_cache` (líneas 154-162) por:

```rust
/// Eviction LRU por tamaño/TTL. Lee los ajustes en caliente y no borra ni las
/// sesiones activas ni lo guardado en la biblioteca (por `info_hash`).
async fn evict_cache(state: &pistreaming_api::SharedState) -> anyhow::Result<()> {
    use std::time::{Duration, SystemTime};

    let cache_max_gb = state.settings.cache_max_gb();
    let cache_ttl_hours = state.settings.cache_ttl_hours();
    let ttl = Duration::from_secs(cache_ttl_hours * 3600);
```

**3c)** Reemplazar la construcción del set de exclusión (líneas 196-197) por:

```rust
    let mut keep_hashes: std::collections::HashSet<String> =
        state.sessions.active_info_hashes().into_iter().collect();
    match state.store.library_info_hashes() {
        Ok(hs) => keep_hashes.extend(hs),
        Err(e) => tracing::warn!(error = %e, "no se pudo leer library para la limpieza"),
    }
```

**3d)** Reemplazar la comparación del guard (líneas 208-211) por:

```rust
        let name = entry.file_name().to_string_lossy().to_string();
        if keep_hashes.contains(&name) {
            continue; // sesión activa o guardada en la biblioteca
        }
```

- [ ] **Step 4: Correr y ver pasar**

Run: `cd ~/Proyectos/PiStreaming && cargo test -p pistreaming-server`

Expected: PASS (tests de `Config` + `evict_cache_no_borra_sesion_activa` + los 2 nuevos).

- [ ] **Step 5: Commit**

```bash
git add crates/server/src/main.rs
git commit -m "feat(server): eviction lee settings al vuelo y excluye la biblioteca"
```

---

## Task 4: `api` — campos de `keep` en `PlaySession`

Categoría sugerida: `quick`.

**Files:**
- Modify: `crates/api/src/session.rs` (`PlaySession` 11-22; `sess()` 103-114)
- Modify: `crates/api/src/lib.rs` (`play()` 401-410)
- Modify: `crates/api/tests/play_progress.rs` (literal 36-48)
- Modify: `crates/server/src/main.rs` (literal de test 329-338)

- [ ] **Step 1: Escribir el test que falla**

En `crates/api/src/session.rs`, dentro del `mod tests`, agregar tras `stale_ids_detecta_viejas` (línea 136):

```rust
    #[test]
    fn sess_conserva_la_identidad_para_keep() {
        let mut s = sess("k");
        s.kind = Some("movie".into());
        s.meta_id = Some("tt1".into());
        s.title = Some("Dune".into());
        s.media_path = Some("/data/cache/mm.mkv".into());
        assert_eq!(s.kind.as_deref(), Some("movie"));
        assert_eq!(s.meta_id.as_deref(), Some("tt1"));
        assert_eq!(s.title.as_deref(), Some("Dune"));
        assert_eq!(
            s.media_path.as_deref(),
            Some(std::path::Path::new("/data/cache/mm.mkv"))
        );
    }
```

> El campo de meta se llama `meta_id` (no `id`) para no colisionar con `PlaySession.id` (el id de la sesión). El handler lo mapea a la ficha como `id`.

- [ ] **Step 2: Correr y ver el fallo**

Run: `cd ~/Proyectos/PiStreaming && cargo test -p pistreaming-api session::tests::sess_conserva`

Expected: FAIL de compilación: `no field kind/meta_id/title/media_path on type PlaySession`.

- [ ] **Step 3: Implementación mínima**

**3a)** En `crates/api/src/session.rs`, agregar los campos al final de `struct PlaySession` (después de `progress_key`, línea 21):

```rust
    /// Tipo de meta (`movie`/`series`) para la ficha de biblioteca.
    pub kind: Option<String>,
    /// Id de meta para la ficha de biblioteca (el `id` del `AppState` es de sesión).
    pub meta_id: Option<String>,
    /// Título de la ficha (cae a `meta_id` si falta).
    pub title: Option<String>,
    /// Ruta local del `.mkv`/`.mp4` final dentro de la caché de librqbit.
    pub media_path: Option<std::path::PathBuf>,
```

**3b)** Actualizar `sess()` (líneas 103-114) para incluir los campos:

```rust
    fn sess(sid: &str) -> PlaySession {
        PlaySession {
            id: sid.into(),
            info_hash: "abc".into(),
            file_id: 0,
            plan: plan(),
            cache_dir: "/tmp".into(),
            created_at: Instant::now(),
            ffmpeg: None,
            progress_key: None,
            kind: None,
            meta_id: None,
            title: None,
            media_path: None,
        }
    }
```

**3c)** En `crates/api/src/lib.rs`, dentro de `play()` reemplazar el bloque de `PlaySession` (líneas 401-410) por (se captura `media_path` antes de mover el handle):

```rust
    let media_path = local_probe_path(&added.handle, file_id);
    st.sessions.insert(PlaySession {
        id: session_id.clone(),
        info_hash: added.info_hash.clone(),
        file_id,
        plan: plan.clone(),
        cache_dir,
        created_at: std::time::Instant::now(),
        ffmpeg: None,
        progress_key: progress_key.clone(),
        kind: body.kind.clone(),
        meta_id: body.id.clone(),
        title: body.title.clone(),
        media_path,
    });
```

**3d)** En `crates/api/tests/play_progress.rs`, reemplazar el literal (líneas 36-48) por:

```rust
    st.sessions.insert(PlaySession {
        id: session.into(),
        info_hash: "dd8255ecdc7c".into(),
        file_id: 1,
        plan: plan_con_progress_url(
            session,
            Some("http://127.0.0.1:8000/api/progress/movie/tt123".into()),
        ),
        cache_dir: tmp.path().to_path_buf(),
        created_at: Instant::now(),
        ffmpeg: None,
        progress_key: Some("movie:tt123".into()),
        kind: Some("movie".into()),
        meta_id: Some("tt123".into()),
        title: Some("Dune".into()),
        media_path: None,
    });
```

**3e)** En `crates/server/src/main.rs`, dentro del test, reemplazar el literal de `PlaySession` (líneas 329-338) por:

```rust
        state.sessions.insert(PlaySession {
            id: active_id,
            info_hash: active_hash.into(),
            file_id: 0,
            plan,
            cache_dir: active_dir.clone(),
            created_at: Instant::now(),
            ffmpeg: None,
            progress_key: None,
            kind: None,
            meta_id: None,
            title: None,
            media_path: None,
        });
```

- [ ] **Step 4: Correr y ver pasar**

Run: `cd ~/Proyectos/PiStreaming && cargo test -p pistreaming-api -p pistreaming-server`

Expected: PASS (compila `session.rs`, `lib.rs`, `play_progress.rs`, `main.rs` y pasan los tests).

- [ ] **Step 5: Commit**

```bash
git add crates/api/src/session.rs crates/api/src/lib.rs crates/api/tests/play_progress.rs crates/server/src/main.rs
git commit -m "feat(api): campos kind/meta_id/title/media_path en PlaySession para keep"
```

---
## Task 5: `api` — `assets.rs` con `rust-embed`, rutas estáticas y retiro de `player.html`

Categoría sugerida: `deep`. Skills: ninguna (infra Rust + HTTP).

**Files:**
- Create: `crates/api/src/assets.rs`
- Create: `crates/api/assets/index.html`
- Create: `crates/api/assets/icon.svg`
- Delete: `crates/api/assets/player.html`
- Modify: `Cargo.toml` (`[workspace.dependencies]` tras línea 46)
- Modify: `crates/api/Cargo.toml` (`[dependencies]` tras línea 25)
- Modify: `crates/api/src/lib.rs` (router 51-69; `PLAYER_HTML` 246; handler `player` 248-255)
- Modify: `crates/api/tests/player.rs` (todo el archivo)

- [ ] **Step 1: Escribir el test que falla**

**1a)** Crear `crates/api/assets/index.html`:

```html
<!doctype html>
<html lang="es">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>PiStreaming</title>
  <link rel="icon" href="/icon.svg" type="image/svg+xml">
  <link rel="stylesheet" href="/assets/style.css">
</head>
<body>
  <header class="topbar">
    <a class="brand" href="#/search">Pi<span>Streaming</span></a>
    <nav id="nav">
      <a href="#/search" data-tab="search">Buscar</a>
      <a href="#/library" data-tab="library">Biblioteca</a>
      <a href="#/addons" data-tab="addons">Addons</a>
      <a href="#/settings" data-tab="settings">Ajustes</a>
    </nav>
  </header>
  <div id="banner" class="banner" hidden></div>
  <main id="app" class="app"></main>
  <script src="/assets/app.js"></script>
</body>
</html>
```

**1b)** Crear `crates/api/assets/icon.svg`:

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">
  <rect width="32" height="32" rx="7" fill="#0b0d10"/>
  <path d="M11 9.5v13l11-6.5z" fill="#e11d48"/>
</svg>
```

**1c)** Crear `crates/api/src/assets.rs`:

```rust
//! Assets estáticos de la SPA, embebidos con `rust-embed`.
//!
//! En debug lee del disco (`CARGO_MANIFEST_DIR/assets`), así editar `app.js` y
//! recargar no requiere recompilar; en release embebe los bytes en el binario.

use axum::body::Body;
use axum::extract::Path;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets/"]
struct Assets;

const INDEX: &str = "index.html";

/// GET / — shell de la SPA.
pub async fn index() -> Response {
    serve(INDEX).unwrap_or_else(|| {
        (StatusCode::INTERNAL_SERVER_ERROR, "index.html no embebido").into_response()
    })
}

/// GET /icon.svg — favicon.
pub async fn icon() -> Response {
    serve("icon.svg").unwrap_or_else(|| StatusCode::NOT_FOUND.into_response())
}

/// GET /assets/*path — `app.js` / `style.css` con su Content-Type.
pub async fn asset(Path(path): Path<String>) -> Response {
    serve(&path).unwrap_or_else(|| StatusCode::NOT_FOUND.into_response())
}

/// GET /play/:session — compatibilidad: redirige a la SPA con hash-routing.
pub async fn play_redirect(Path(session): Path<String>) -> Response {
    let location = format!("/#/player/{session}");
    (
        StatusCode::TEMPORARY_REDIRECT,
        [(
            header::LOCATION,
            HeaderValue::from_str(&location).unwrap_or(HeaderValue::from_static("/")),
        )],
    )
        .into_response()
}

fn serve(path: &str) -> Option<Response> {
    let file = Assets::get(path)?;
    let mime = mime_for(path);
    let mut resp = Response::new(Body::from(file.data.into_owned()));
    *resp.status_mut() = StatusCode::OK;
    resp.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache_control(path)));
    Some(resp)
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

fn cache_control(path: &str) -> &'static str {
    if path == INDEX {
        "no-cache"
    } else {
        "public, max-age=3600"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_e_icon_estan_embebidos() {
        assert!(Assets::get("index.html").is_some(), "falta crates/api/assets/index.html");
        assert!(Assets::get("icon.svg").is_some(), "falta crates/api/assets/icon.svg");
    }

    #[test]
    fn mime_por_extension() {
        assert_eq!(mime_for("index.html"), "text/html; charset=utf-8");
        assert_eq!(mime_for("app.js"), "text/javascript; charset=utf-8");
        assert_eq!(mime_for("style.css"), "text/css; charset=utf-8");
        assert_eq!(mime_for("icon.svg"), "image/svg+xml");
    }

    #[test]
    fn index_no_se_cachea() {
        assert_eq!(cache_control("index.html"), "no-cache");
        assert_eq!(cache_control("app.js"), "public, max-age=3600");
    }
}
```

**1d)** Reemplazar todo `crates/api/tests/player.rs` por:

```rust
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

/// `/play/:session` ya no sirve HTML propio: redirige a la SPA (`#/player/:session`).
#[tokio::test]
async fn play_redirige_a_la_spa() {
    let tmp = tempfile::tempdir().unwrap();
    let app = pistreaming_api::router(pistreaming_api::test_state(tmp.path().to_path_buf()));
    let res = app
        .oneshot(
            Request::builder()
                .uri("/play/abc-0")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        res.headers().get("location").unwrap(),
        "/#/player/abc-0",
        "debe apuntar al hash-routing de la SPA"
    );
}
```

- [ ] **Step 2: Correr y ver el fallo**

Run: `cd ~/Proyectos/PiStreaming && cargo test -p pistreaming-api assets::`

Expected: FAIL de compilación: `unresolved import rust_embed` / `cannot find derive macro RustEmbed`. (Aún no está la dep.)

- [ ] **Step 3: Implementación mínima**

**3a)** En `Cargo.toml` (workspace), tras la línea 46 (`http = "1"`), agregar:

```toml
rust-embed = "8"
```

**3b)** En `crates/api/Cargo.toml`, tras la línea 25 (`parking_lot.workspace = true`), agregar:

```toml
rust-embed.workspace = true
```

**3c)** En `crates/api/src/lib.rs`, eliminar por completo el bloque del player (líneas 246-255):

```rust
const PLAYER_HTML: &str = include_str!("../assets/player.html");

/// GET /play/:session — página mínima del reproductor.
pub async fn player(Path(_session): Path<String>) -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        PLAYER_HTML,
    )
        .into_response()
}
```

**3d)** En `router` (líneas 51-69), reemplazar la línea

```rust
        .route("/play/:session", get(player))
```

por las rutas estáticas y el redirect:

```rust
        .route("/", get(assets::index))
        .route("/assets/*path", get(assets::asset))
        .route("/icon.svg", get(assets::icon))
        .route("/play/:session", get(assets::play_redirect))
```

**3e)** Eliminar el archivo `crates/api/assets/player.html`:

```bash
rm crates/api/assets/player.html
```

- [ ] **Step 4: Correr y ver pasar**

Run: `cd ~/Proyectos/PiStreaming && cargo test -p pistreaming-api assets:: && cargo test -p pistreaming-api --test player`

Expected: PASS (3 tests inline de `assets` + `play_redirige_a_la_spa`).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/api/Cargo.toml crates/api/src/assets.rs crates/api/src/lib.rs crates/api/assets/index.html crates/api/assets/icon.svg crates/api/assets/player.html crates/api/tests/player.rs
git commit -m "feat(api): SPA embebida con rust-embed, rutas estaticas y retiro de player.html"
```

---
## Task 6: `api` — endpoints de biblioteca y `/library/:id/stream`

Categoría sugerida: `deep`.

**Files:**
- Create: `crates/api/src/library.rs`
- Create: `crates/api/tests/library_api.rs`
- Modify: `crates/api/src/lib.rs` (router 51-69; `decode_url` 242)

- [ ] **Step 1: Escribir el test que falla**

Crear `crates/api/tests/library_api.rs` con este contenido completo:

```rust
//! API de la biblioteca: `GET|POST|DELETE /api/library` y `GET /library/:id/stream`.
//!
//! Como `POST /api/library` exige un torrent descargado (handle vivo + `stats().finished`),
//! la ruta feliz se siembra con `Store::keep` sobre un archivo real y se verifica el
//! contrato HTTP; el chequeo de completitud se cubre con los casos 404/409.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use pistreaming_api::session::PlaySession;
use pistreaming_core::playback::{PlaybackPlan, PlaybackRoute};
use std::time::Instant;
use tower::ServiceExt;

fn plan(session: &str) -> PlaybackPlan {
    PlaybackPlan {
        session: session.into(),
        route: PlaybackRoute::Direct,
        playback_url: format!("/stream/{session}"),
        raw_url: Some(format!("/raw/{session}")),
        browser_may_fail: false,
        needs_recode_audio: false,
        video_codec: "h264".into(),
        audio_codec: Some("aac".into()),
        progress_url: None,
    }
}

fn seed_library(
    st: &pistreaming_api::SharedState,
    kind: &str,
    id: &str,
    bytes: &[u8],
) -> String {
    let src = st.cache_dir.join(format!("{kind}_{id}.mkv"));
    std::fs::write(&src, bytes).unwrap();
    st.store
        .keep(&st.library_dir, kind, id, "Título", Some("hashZ"), &src)
        .unwrap()
        .file_path
}

#[tokio::test]
async fn get_library_lista_y_delete_es_idempotente() {
    let tmp = tempfile::tempdir().unwrap();
    let st = pistreaming_api::test_state(tmp.path().to_path_buf());
    let file_path = seed_library(&st, "movie", "tt1", b"0123456789");
    let app = pistreaming_api::router(st.clone());

    let res = app
        .clone()
        .oneshot(Request::get("/api/library").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["items"][0]["id"], "movie:tt1");
    assert_eq!(v["items"][0]["size_bytes"], 10);

    let res = app
        .clone()
        .oneshot(
            Request::delete("/api/library/movie:tt1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert!(!std::path::Path::new(&file_path).exists(), "borra el archivo");

    // Idempotente: repetir sigue dando 204.
    let res = app
        .clone()
        .oneshot(
            Request::delete("/api/library/movie:tt1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    let res = app
        .oneshot(Request::get("/api/library").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["items"], serde_json::json!([]));
}

#[tokio::test]
async fn post_library_sin_sesion_es_404() {
    let tmp = tempfile::tempdir().unwrap();
    let app = pistreaming_api::router(pistreaming_api::test_state(tmp.path().to_path_buf()));
    let res = app
        .oneshot(
            Request::post("/api/library")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"session":"no-existe"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn post_library_con_descarga_incompleta_es_409() {
    let tmp = tempfile::tempdir().unwrap();
    let st = pistreaming_api::test_state(tmp.path().to_path_buf());
    let media = tmp.path().join("c.mkv");
    std::fs::write(&media, b"x").unwrap();
    st.sessions.insert(PlaySession {
        id: "sess-0".into(),
        info_hash: "hash".into(),
        file_id: 0,
        plan: plan("sess-0"),
        cache_dir: tmp.path().to_path_buf(),
        created_at: Instant::now(),
        ffmpeg: None,
        progress_key: Some("movie:tt9".into()),
        kind: Some("movie".into()),
        meta_id: Some("tt9".into()),
        title: Some("Peli".into()),
        media_path: Some(media),
    });
    let app = pistreaming_api::router(st);

    let res = app
        .oneshot(
            Request::post("/api/library")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"session":"sess-0"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::CONFLICT,
        "sin handle vivo => descarga incompleta"
    );
}

#[tokio::test]
async fn library_stream_sirve_rango_206() {
    let tmp = tempfile::tempdir().unwrap();
    let st = pistreaming_api::test_state(tmp.path().to_path_buf());
    seed_library(&st, "movie", "tt2", b"0123456789");
    let app = pistreaming_api::router(st);

    let res = app
        .oneshot(
            Request::get("/library/movie:tt2/stream")
                .header("range", "bytes=2-4")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&body[..], b"234");
}

#[tokio::test]
async fn library_stream_id_inexistente_es_404() {
    let tmp = tempfile::tempdir().unwrap();
    let app = pistreaming_api::router(pistreaming_api::test_state(tmp.path().to_path_buf()));
    let res = app
        .oneshot(
            Request::get("/library/movie:nope/stream")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 2: Correr y ver el fallo**

Run: `cd ~/Proyectos/PiStreaming && cargo test -p pistreaming-api --test library_api`

Expected: FAIL de compilación: `unresolved module library` en el crate y `unresolved import pistreaming_api::session` si el módulo no está. (`SharedState` ya es `pub`.)

- [ ] **Step 3: Implementación mínima**

**3a)** En `crates/api/src/lib.rs` (línea 242), reemplazar

```rust
fn decode_url(s: &str) -> String {
```

por

```rust
pub(crate) fn decode_url(s: &str) -> String {
```

**3b)** Crear `crates/api/src/library.rs`:

```rust
//! Endpoints de la biblioteca permanente (spec §5.1).

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use pistreaming_core::error::CoreError;
use serde::Deserialize;

use crate::SharedState;

#[derive(Deserialize)]
pub struct KeepBody {
    pub session: String,
}

/// GET /api/library
pub async fn list(State(st): State<SharedState>) -> Response {
    match st.store.list_library() {
        Ok(items) => Json(serde_json::json!({ "items": items })).into_response(),
        Err(e) => crate::core_err(e),
    }
}

/// POST /api/library { session } -> 201 + ficha (dispara el `keep`).
pub async fn keep(
    State(st): State<SharedState>,
    body: Result<Json<KeepBody>, JsonRejection>,
) -> Response {
    let Json(body) = match body {
        Ok(b) => b,
        Err(_) => return crate::err(StatusCode::BAD_REQUEST, "body inválido"),
    };
    let Some(sess) = st.sessions.get(&body.session) else {
        return crate::err(StatusCode::NOT_FOUND, "sesión desconocida");
    };
    // Completitud: el torrent debe estar descargado (`stats().finished`).
    let handle = st.handles.read().get(&body.session).cloned();
    let finished = handle.map(|h| h.stats().finished).unwrap_or(false);
    if !finished {
        return crate::err(StatusCode::CONFLICT, "la descarga no está completa");
    }

    let (info_hash, kind, meta_id, title, media_path) = {
        let g = sess.read();
        (
            g.info_hash.clone(),
            g.kind.clone(),
            g.meta_id.clone(),
            g.title.clone(),
            g.media_path.clone(),
        )
    };
    let (Some(kind), Some(id), Some(media_path)) = (kind, meta_id, media_path) else {
        return crate::err(
            StatusCode::CONFLICT,
            "la sesión no tiene identidad de ficha (kind/id)",
        );
    };
    let title = title.unwrap_or_else(|| id.clone());
    let info_hash_opt = if info_hash.is_empty() { None } else { Some(info_hash) };

    match st
        .store
        .keep(&st.library_dir, &kind, &id, &title, info_hash_opt.as_deref(), &media_path)
    {
        Ok(item) => (StatusCode::CREATED, Json(item)).into_response(),
        Err(CoreError::NotFound(_)) => {
            crate::err(StatusCode::CONFLICT, "el archivo de la sesión no está disponible")
        }
        Err(e) => crate::core_err(e),
    }
}

/// DELETE /api/library/:id — idempotente.
pub async fn delete(State(st): State<SharedState>, Path(id): Path<String>) -> Response {
    let id = crate::decode_url(&id);
    match st.store.remove_library(&id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => crate::core_err(e),
    }
}

/// GET /library/:id/stream — sirve el archivo guardado con soporte de Range.
pub async fn stream(
    State(st): State<SharedState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let id = crate::decode_url(&id);
    let item = match st.store.list_library() {
        Ok(items) => items.into_iter().find(|i| i.id == id),
        Err(e) => return crate::core_err(e),
    };
    let Some(item) = item else {
        return crate::err(StatusCode::NOT_FOUND, "no está en la biblioteca");
    };
    let path = std::path::PathBuf::from(&item.file_path);
    let file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return crate::err(StatusCode::NOT_FOUND, "archivo no encontrado")
        }
        Err(e) => {
            return crate::core_err(CoreError::Other(format!(
                "no se pudo abrir {}: {e}",
                path.display()
            )))
        }
    };
    let len = match file.metadata().await {
        Ok(m) => m.len(),
        Err(e) => return crate::core_err(CoreError::Other(format!("no se pudo medir: {e}"))),
    };
    crate::range::ranged_response(file, len, &headers, mime_for_path(&path)).await
}

fn mime_for_path(p: &std::path::Path) -> &'static str {
    match p
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("mkv") => "video/x-matroska",
        Some("mp4") | Some("m4v") => "video/mp4",
        Some("webm") => "video/webm",
        Some("avi") => "video/x-msvideo",
        Some("mov") => "video/quicktime",
        Some("ts") => "video/mp2t",
        _ => "application/octet-stream",
    }
}
```

**3c)** En `router` (`crates/api/src/lib.rs`), agregar antes de `.with_state(state)`:

```rust
        .route("/api/library", get(library::list).post(library::keep))
        .route("/api/library/:id", axum::routing::delete(library::delete))
        .route("/library/:id/stream", get(library::stream))
```

- [ ] **Step 4: Correr y ver pasar**

Run: `cd ~/Proyectos/PiStreaming && cargo test -p pistreaming-api --test library_api`

Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/api/src/library.rs crates/api/src/lib.rs crates/api/tests/library_api.rs
git commit -m "feat(api): endpoints de biblioteca y /library/:id/stream con Range"
```

---
## Task 7: `api` — `PATCH /api/addons/:url`

Categoría sugerida: `quick`.

**Files:**
- Modify: `crates/api/src/lib.rs` (router 55; nuevo handler tras `remove_addon` línea 159; `mod tests`)
- Test: tests inline en `crates/api/src/lib.rs`

- [ ] **Step 1: Escribir el test que falla**

En `crates/api/src/lib.rs`, dentro del `mod tests`, agregar tras `add_addon_invalid_body_is_400` (línea 912). Usa el helper `enc` ya existente en ese módulo:

```rust
    #[tokio::test]
    async fn patch_addon_togglea_enabled() {
        let app = router(test_state().await);

        // La URL del addon mock se descubre desde el listado.
        let res = app
            .clone()
            .oneshot(Request::get("/api/addons").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
        let url = rows[0]["url"].as_str().unwrap().to_string();
        assert_eq!(rows[0]["enabled"], true);

        // Desactivar sin borrar.
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/addons/{}", enc(&url)))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"enabled":false}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["enabled"], false);

        // Sigue listado, pero deshabilitado.
        let res = app
            .clone()
            .oneshot(Request::get("/api/addons").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(rows.len(), 1, "desactivar no debe borrar");
        assert_eq!(rows[0]["enabled"], false);

        // Reactivar.
        let res = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/addons/{}", enc(&url)))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"enabled":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn patch_addon_url_desconocida_es_404() {
        let app = router(test_state().await);
        let res = app
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/addons/{}", enc("http://127.0.0.1:1")))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"enabled":false}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
```

- [ ] **Step 2: Correr y ver el fallo**

Run: `cd ~/Proyectos/PiStreaming && cargo test -p pistreaming-api patch_addon`

Expected: FAIL: `405 Method Not Allowed` (la ruta `/api/addons/:url` solo admite DELETE).

- [ ] **Step 3: Implementación mínima**

**3a)** En `crates/api/src/lib.rs`, justo después de `remove_addon` (línea 159), agregar:

```rust
#[derive(Deserialize)]
pub struct PatchAddonBody {
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// PATCH /api/addons/:url { enabled? } — activa/desactiva sin borrar.
pub async fn patch_addon(
    State(st): State<SharedState>,
    Path(url): Path<String>,
    body: Result<Json<PatchAddonBody>, JsonRejection>,
) -> Response {
    let Json(body) = match body {
        Ok(b) => b,
        Err(_) => return err(StatusCode::BAD_REQUEST, "body inválido"),
    };
    let Some(enabled) = body.enabled else {
        return err(StatusCode::BAD_REQUEST, "falta `enabled`");
    };
    let url = normalize_url(&decode_url(&url));
    let _guard = st.mutex.lock().await;
    if let Err(e) = st.store.set_addon_enabled(&url, enabled) {
        return core_err(e);
    }
    match AddonManager::load(st.client.clone(), &st.store).await {
        Ok(fresh) => {
            let updated = fresh
                .addons()
                .iter()
                .find(|a| a.url == url)
                .map(|a| {
                    serde_json::json!({
                        "url": a.url,
                        "name": a.manifest.name,
                        "enabled": a.enabled,
                    })
                });
            *st.addons.write().await = fresh;
            match updated {
                Some(v) => Json(v).into_response(),
                None => err(StatusCode::NOT_FOUND, "no encontrado"),
            }
        }
        Err(e) => core_err(e),
    }
}
```

**3b)** En `router` (línea 55), reemplazar

```rust
        .route("/api/addons/:url", axum::routing::delete(remove_addon))
```

por

```rust
        .route(
            "/api/addons/:url",
            axum::routing::delete(remove_addon).patch(patch_addon),
        )
```

- [ ] **Step 4: Correr y ver pasar**

Run: `cd ~/Proyectos/PiStreaming && cargo test -p pistreaming-api patch_addon`

Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/api/src/lib.rs
git commit -m "feat(api): PATCH /api/addons/:url para activar/desactivar sin borrar"
```

---
## Task 8: `api` — tema y hoja de estilos de la SPA (`style.css`)

Categoría sugerida: `visual-engineering`. Skills: usar `@designer` para el acabado visual si está disponible; el contrato (rutas y clases CSS) no cambia.

**Files:**
- Create: `crates/api/assets/style.css`
- Create: `crates/api/tests/spa_assets.rs`

- [ ] **Step 1: Escribir el test que falla**

Crear `crates/api/tests/spa_assets.rs` con este contenido completo:

```rust
//! La SPA embebida se sirve con su Content-Type (spec §8).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn index_sirve_el_shell_de_la_spa() {
    let tmp = tempfile::tempdir().unwrap();
    let app = pistreaming_api::router(pistreaming_api::test_state(tmp.path().to_path_buf()));
    let res = app
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let ct = res.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(ct.starts_with("text/html"), "content-type html: {ct}");
    let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("id=\"app\""), "shell con #app: {html}");
    assert!(html.contains("/assets/app.js"), "shell enlaza app.js: {html}");
}

#[tokio::test]
async fn assets_sirven_css_con_su_content_type() {
    let tmp = tempfile::tempdir().unwrap();
    let app = pistreaming_api::router(pistreaming_api::test_state(tmp.path().to_path_buf()));
    let res = app
        .clone()
        .oneshot(Request::get("/assets/style.css").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("text/css"));

    let res = app
        .oneshot(Request::get("/icon.svg").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("image/svg+xml"));
}

#[tokio::test]
async fn asset_inexistente_es_404() {
    let tmp = tempfile::tempdir().unwrap();
    let app = pistreaming_api::router(pistreaming_api::test_state(tmp.path().to_path_buf()));
    let res = app
        .oneshot(Request::get("/assets/nope.xyz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 2: Correr y ver el fallo**

Run: `cd ~/Proyectos/PiStreaming && cargo test -p pistreaming-api --test spa_assets`

Expected: FAIL en `assets_sirven_css_con_su_content_type`: `/assets/style.css` devuelve `404`. (`index_sirve_el_shell_de_la_spa` y `asset_inexistente_es_404` ya pasan.)

- [ ] **Step 3: Implementación mínima**

Crear `crates/api/assets/style.css` con este contenido completo (tema «cine oscuro — negro + rosa»):

```css
:root {
  --bg: #0b0d10;
  --surface: #15181d;
  --surface-2: #1d2128;
  --text: #e8eaed;
  --muted: #9aa3ad;
  --accent: #e11d48;
  --accent-2: #fb7185;
  --radius: 12px;
}

* { box-sizing: border-box; }

body {
  margin: 0;
  background: var(--bg);
  color: var(--text);
  font-family: system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
  line-height: 1.45;
}

a { color: inherit; text-decoration: none; }

.topbar {
  position: sticky;
  top: 0;
  z-index: 10;
  display: flex;
  align-items: center;
  gap: 20px;
  padding: 12px 20px;
  background: rgba(11, 13, 16, 0.92);
  backdrop-filter: blur(8px);
  border-bottom: 1px solid var(--surface-2);
}

.brand { font-weight: 700; letter-spacing: 0.2px; }
.brand span { color: var(--accent); }

#nav { display: flex; gap: 6px; flex-wrap: wrap; }
#nav a {
  padding: 6px 12px;
  border-radius: 999px;
  color: var(--muted);
  font-size: 14px;
}
#nav a:hover { color: var(--text); background: var(--surface); }
#nav a.active { color: #fff; background: var(--accent); }

.banner {
  margin: 10px 20px 0;
  padding: 10px 14px;
  border-radius: var(--radius);
  background: var(--surface-2);
  border-left: 4px solid var(--accent);
  font-size: 14px;
}

.app { padding: 20px; max-width: 1100px; margin: 0 auto; }
h1 { font-size: 22px; margin: 4px 0 16px; }
h2.section {
  font-size: 16px;
  color: var(--muted);
  text-transform: uppercase;
  letter-spacing: 1px;
  margin: 24px 0 12px;
}
.muted { color: var(--muted); font-size: 13px; }
.desc { color: var(--text); max-width: 70ch; }

.searchbar { display: flex; gap: 10px; margin: 12px 0 20px; }
.searchbar input, .field input {
  flex: 1;
  padding: 10px 14px;
  border-radius: var(--radius);
  border: 1px solid var(--surface-2);
  background: var(--surface);
  color: var(--text);
  font-size: 15px;
}
.searchbar input:focus, .field input:focus {
  outline: 2px solid var(--accent);
  border-color: transparent;
}

.btn {
  padding: 10px 16px;
  border-radius: var(--radius);
  border: 1px solid var(--surface-2);
  background: var(--surface);
  color: var(--text);
  cursor: pointer;
  font-size: 14px;
}
.btn:hover { background: var(--surface-2); }
.btn.primary { background: var(--accent); border-color: var(--accent); color: #fff; }
.btn.primary:hover { background: var(--accent-2); border-color: var(--accent-2); }
.btn.ghost { background: transparent; color: var(--muted); }
.btn:disabled { opacity: 0.5; cursor: default; }

.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
  gap: 16px;
}

.card {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px;
  background: var(--surface);
  border-radius: var(--radius);
  transition: transform 0.12s ease, background 0.12s ease;
}
.card:hover { transform: translateY(-2px); background: var(--surface-2); }
.card-title { font-size: 14px; font-weight: 600; }
.card-actions { display: flex; gap: 8px; margin-top: auto; }

.poster {
  aspect-ratio: 2 / 3;
  border-radius: 8px;
  background: var(--surface-2) center/cover no-repeat;
  display: flex;
  align-items: center;
  justify-content: center;
}
.poster.large { width: 180px; min-width: 180px; aspect-ratio: 2 / 3; }
.poster-fallback { font-size: 40px; font-weight: 700; color: var(--accent); }

.detail-head { display: flex; gap: 20px; align-items: flex-start; }
.detail-meta h1 { margin-top: 0; }

.list { display: flex; flex-direction: column; gap: 10px; }
.row-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 14px;
  background: var(--surface);
  border-radius: var(--radius);
}

.switch { display: flex; align-items: center; gap: 6px; color: var(--muted); font-size: 14px; }
.switch input { accent-color: var(--accent); width: 18px; height: 18px; }

.form { display: flex; flex-direction: column; gap: 14px; max-width: 520px; }
.field { display: flex; flex-direction: column; gap: 6px; font-size: 14px; }
.field small { font-size: 12px; }

.player-wrap { display: flex; flex-direction: column; gap: 12px; }
.player-title { font-size: 18px; }
.video { width: 100%; max-height: 78vh; background: #000; border-radius: var(--radius); }
.player-actions { display: flex; gap: 10px; }

.state {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 10px;
  padding: 40px 16px;
  color: var(--muted);
  text-align: center;
}
.state.error { color: var(--accent-2); flex-direction: column; }

.spinner {
  width: 18px;
  height: 18px;
  border: 2px solid var(--surface-2);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.8s linear infinite;
}
@keyframes spin { to { transform: rotate(360deg); } }

@media (max-width: 640px) {
  .detail-head { flex-direction: column; }
  .poster.large { width: 100%; }
  .app { padding: 14px; }
}
```

- [ ] **Step 4: Correr y ver pasar**

Run: `cd ~/Proyectos/PiStreaming && cargo test -p pistreaming-api --test spa_assets`

Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/api/assets/style.css crates/api/tests/spa_assets.rs
git commit -m "feat(api): hoja de estilos de la SPA con tema cine oscuro"
```

---
## Task 9: `api` — router y vistas de la SPA (`app.js`)

Categoría sugerida: `visual-engineering`. Skills: usar `@designer` para el acabado visual si está disponible; el contrato de endpoints (Tasks 6/7) y las clases CSS (Task 8) no cambian.

**Files:**
- Create: `crates/api/assets/app.js`
- Modify: `crates/api/tests/spa_assets.rs` (agregar el test de `app.js`)

- [ ] **Step 1: Escribir el test que falla**

En `crates/api/tests/spa_assets.rs`, agregar al final del archivo:

```rust
#[tokio::test]
async fn assets_sirven_js_con_su_content_type() {
    let tmp = tempfile::tempdir().unwrap();
    let app = pistreaming_api::router(pistreaming_api::test_state(tmp.path().to_path_buf()));
    let res = app
        .oneshot(Request::get("/assets/app.js").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("text/javascript"));
}
```

- [ ] **Step 2: Correr y ver el fallo**

Run: `cd ~/Proyectos/PiStreaming && cargo test -p pistreaming-api --test spa_assets assets_sirven_js`

Expected: FAIL: `/assets/app.js` devuelve `404`.

- [ ] **Step 3: Implementación mínima**

Crear `crates/api/assets/app.js` con este contenido completo:

```js
// PiStreaming SPA — router por hash + vistas. Sin dependencias ni build.
const app = document.getElementById("app");
const nav = document.getElementById("nav");
const banner = document.getElementById("banner");

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v == null || v === false) continue;
    if (k === "class") node.className = v;
    else if (k.startsWith("on") && typeof v === "function") node.addEventListener(k.slice(2), v);
    else if (v === true) node.setAttribute(k, "");
    else node.setAttribute(k, v);
  }
  for (const c of children.flat()) {
    if (c == null || c === false) continue;
    node.append(c.nodeType ? c : document.createTextNode(String(c)));
  }
  return node;
}

function showBanner(msg) {
  banner.textContent = msg;
  banner.hidden = false;
  clearTimeout(showBanner._t);
  showBanner._t = setTimeout(() => { banner.hidden = true; }, 6000);
}

async function api(path, opts = {}) {
  const res = await fetch(path, opts);
  if (!res.ok) {
    let msg = `Error ${res.status}`;
    try {
      const j = await res.json();
      if (j.error) msg = j.error;
    } catch (_) {}
    throw new Error(msg);
  }
  if (res.status === 204) return null;
  const ct = res.headers.get("content-type") || "";
  return ct.includes("application/json") ? res.json() : res.text();
}

function setTab(tab) {
  if (!nav) return;
  for (const a of nav.querySelectorAll("a")) a.classList.toggle("active", a.dataset.tab === tab);
}

const loading = () => el("div", { class: "state" }, el("div", { class: "spinner" }), "Cargando…");
const empty = (msg) => el("div", { class: "state" }, msg);
function errorBox(msg, retry) {
  return el("div", { class: "state error" }, el("p", {}, msg),
    retry ? el("button", { class: "btn", onclick: retry }, "Reintentar") : null);
}

async function route() {
  const raw = location.hash.replace(/^#\/?/, "");
  const parts = raw.split("/");
  const head = parts[0] || "search";
  app.replaceChildren(loading());
  try {
    if (head === "search") return await viewSearch();
    if (head === "detail") return await viewDetail(parts[1], parts.slice(2).join("/"));
    if (head === "player") return await viewPlayer(parts[1]);
    if (head === "library") {
      if (parts[1]) return await viewLibraryPlayer(decodeURIComponent(parts.slice(1).join("/")));
      return await viewLibrary();
    }
    if (head === "addons") return await viewAddons();
    if (head === "settings") return await viewSettings();
    app.replaceChildren(empty("Ruta desconocida"));
  } catch (e) {
    showBanner(e.message);
    app.replaceChildren(errorBox(e.message, route));
  }
}

// --- Buscar ---------------------------------------------------------------
async function viewSearch() {
  setTab("search");
  const form = el("form", { class: "searchbar", onsubmit: async (e) => {
    e.preventDefault();
    const q = e.target.q.value.trim();
    if (q) await runSearch(q);
  }},
    el("input", { name: "q", placeholder: "Buscar películas y series…" }),
    el("button", { class: "btn primary", type: "submit" }, "Buscar"),
  );
  app.replaceChildren(el("h1", {}, "Buscar"), form,
    el("div", { id: "results", class: "grid" }));
  const last = sessionStorage.getItem("lastQuery");
  if (last) {
    form.querySelector("[name=q]").value = last;
    await runSearch(last);
  }
}

async function runSearch(q) {
  sessionStorage.setItem("lastQuery", q);
  const results = document.getElementById("results");
  results.replaceChildren(loading());
  const data = await api(`/api/search?query=${encodeURIComponent(q)}`);
  const metas = data.metas || [];
  if (!metas.length) return results.replaceChildren(empty("Sin resultados"));
  results.replaceChildren(...metas.map(metaCard));
}

function metaCard(m) {
  const poster = m.poster || m.background || "";
  return el("a", { class: "card", href: `#/detail/${m.type}/${encodeURIComponent(m.id)}` },
    el("div", { class: "poster", style: poster ? `background-image:url('${poster}')` : "" },
      poster ? null : el("span", { class: "poster-fallback" }, (m.name || "?").slice(0, 1))),
    el("div", { class: "card-title" }, m.name || m.id),
  );
}

// --- Detalle --------------------------------------------------------------
async function viewDetail(kind, id) {
  setTab("search");
  if (!kind || !id) { app.replaceChildren(empty("Ficha inválida")); return; }
  id = decodeURIComponent(id);
  const [meta, streamsData] = await Promise.all([
    api(`/api/meta/${kind}/${encodeURIComponent(id)}`).catch(() => null),
    api(`/api/streams/${kind}/${encodeURIComponent(id)}`).catch(() => ({ streams: [] })),
  ]);
  const m = meta || { name: id };
  const streams = streamsData.streams || [];
  const head = el("div", { class: "detail-head" },
    el("div", { class: "poster large", style: m.poster ? `background-image:url('${m.poster}')` : "" },
      m.poster ? null : el("span", { class: "poster-fallback" }, (m.name || "?").slice(0, 1))),
    el("div", { class: "detail-meta" },
      el("h1", {}, m.name || id),
      m.releaseInfo ? el("p", { class: "muted" }, m.releaseInfo) : null,
      m.description ? el("p", { class: "desc" }, m.description) : null,
    ),
  );
  const list = el("div", { class: "list" });
  if (!streams.length) list.append(empty("Sin fuentes disponibles"));
  streams.forEach((s) => {
    list.append(el("div", { class: "row-item" },
      el("div", {}, el("strong", {}, s.name || "Fuente"),
        el("div", { class: "muted" }, s.title || s.infoHash || s.url || "")),
      el("button", { class: "btn primary", onclick: () => startPlay(s, kind, id, m.name) }, "Reproducir"),
    ));
  });
  app.replaceChildren(head, el("h2", { class: "section" }, "Fuentes"), list);
}

async function startPlay(stream, kind, id, title) {
  const magnet = stream.infoHash ? `magnet:?xt=urn:btih:${stream.infoHash}` : stream.url;
  if (!magnet) { showBanner("La fuente no trae magnet ni url"); return; }
  try {
    const plan = await api("/api/play", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ magnet, title, kind, id }),
    });
    location.hash = `#/player/${encodeURIComponent(plan.session)}`;
  } catch (e) { showBanner(e.message); }
}

// --- Reproductor (torrent) ------------------------------------------------
async function viewPlayer(session) {
  setTab(null);
  session = decodeURIComponent(session || "");
  const plan = await api(`/api/play/${encodeURIComponent(session)}`);
  app.replaceChildren(playerView(plan.playback_url, plan.progress_url, session, null));
}

// --- Reproductor (biblioteca) ---------------------------------------------
async function viewLibraryPlayer(id) {
  setTab("library");
  const data = await api("/api/library");
  const item = (data.items || []).find((i) => i.id === id);
  const [kind, ...rest] = id.split(":");
  const progressUrl = `/api/progress/${kind}/${rest.join(":")}`;
  app.replaceChildren(
    playerView(`/library/${encodeURIComponent(id)}/stream`, progressUrl, null, item ? item.title : id),
  );
}

function playerView(src, progressUrl, session, title) {
  const video = el("video", { class: "video", controls: true, autoplay: true, playsinline: true, src });
  video.addEventListener("error", () => showBanner("El navegador no puede reproducir este archivo."));

  if (progressUrl) {
    fetch(progressUrl)
      .then((r) => (r.ok ? r.json() : null))
      .then((p) => {
        if (p && p.position > 1 && (!p.duration || p.position < p.duration - 15)) {
          video.currentTime = p.position;
        }
      })
      .catch(() => {});
  }

  const saveProgress = () => {
    if (!progressUrl || !video.duration) return;
    fetch(progressUrl, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ position: video.currentTime, duration: video.duration }),
    }).catch(() => {});
  };
  const tick = setInterval(saveProgress, 10000);
  video.addEventListener("pause", saveProgress);
  video.addEventListener("ended", saveProgress);
  window.addEventListener("hashchange", () => {
    saveProgress();
    clearInterval(tick);
  }, { once: true });

  const actions = [el("button", { class: "btn ghost", onclick: () => history.back() }, "Volver")];
  if (session) {
    actions.push(el("button", { class: "btn", onclick: async (e) => {
      e.target.disabled = true;
      try {
        await api("/api/library", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ session }),
        });
        showBanner("Guardado en la biblioteca");
      } catch (err) { showBanner(err.message); }
      finally { e.target.disabled = false; }
    } }, "Guardar"));
  }
  return el("div", { class: "player-wrap" },
    title ? el("h1", { class: "player-title" }, title) : null,
    video,
    el("div", { class: "player-actions" }, ...actions),
  );
}

// --- Biblioteca -----------------------------------------------------------
async function viewLibrary() {
  setTab("library");
  const data = await api("/api/library");
  const items = data.items || [];
  const body = items.length
    ? el("div", { class: "grid" }, ...items.map(libCard))
    : empty("Todavía no guardaste nada");
  app.replaceChildren(el("h1", {}, "Biblioteca"), body);
}

function libCard(it) {
  const mb = Math.round((it.size_bytes || 0) / (1024 * 1024));
  return el("div", { class: "card" },
    el("div", { class: "card-title" }, it.title || it.id),
    el("div", { class: "muted" }, `${it.kind} · ${mb} MB`),
    el("div", { class: "card-actions" },
      el("a", { class: "btn primary", href: `#/library/${encodeURIComponent(it.id)}` }, "Reproducir"),
      el("button", { class: "btn ghost", onclick: async () => {
        try {
          await api(`/api/library/${encodeURIComponent(it.id)}`, { method: "DELETE" });
          route();
        } catch (e) { showBanner(e.message); }
      } }, "Quitar"),
    ),
  );
}

// --- Addons ---------------------------------------------------------------
async function viewAddons() {
  setTab("addons");
  const rows = await api("/api/addons");
  const form = el("form", { class: "searchbar", onsubmit: async (e) => {
    e.preventDefault();
    const url = e.target.url.value.trim();
    if (!url) return;
    try {
      await api("/api/addons", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ url }),
      });
      route();
    } catch (err) { showBanner(err.message); }
  }},
    el("input", { name: "url", placeholder: "https://addon.example/manifest.json" }),
    el("button", { class: "btn primary", type: "submit" }, "Agregar"),
  );
  const list = el("div", { class: "list" });
  if (!rows.length) list.append(empty("Sin addons"));
  rows.forEach((a) => {
    list.append(el("div", { class: "row-item" },
      el("div", {}, el("strong", {}, a.name || a.url), el("div", { class: "muted" }, a.url)),
      el("label", { class: "switch" },
        el("input", { type: "checkbox", checked: !!a.enabled, onchange: async (e) => {
          try {
            await api(`/api/addons/${encodeURIComponent(a.url)}`, {
              method: "PATCH",
              headers: { "content-type": "application/json" },
              body: JSON.stringify({ enabled: e.target.checked }),
            });
          } catch (err) {
            showBanner(err.message);
            e.target.checked = !e.target.checked;
          }
        } }),
        "activo"),
      el("button", { class: "btn ghost", onclick: async () => {
        try {
          await api(`/api/addons/${encodeURIComponent(a.url)}`, { method: "DELETE" });
          route();
        } catch (err) { showBanner(err.message); }
      } }, "Borrar"),
    ));
  });
  app.replaceChildren(el("h1", {}, "Addons"), form, list);
}

// --- Ajustes --------------------------------------------------------------
const HOT_FIELDS = ["cache_max_gb", "cache_ttl_hours"];
const STATIC_FIELDS = ["egress_bind", "http_port", "data_dir"];
const LABELS = {
  cache_max_gb: "Caché máxima (GB)",
  cache_ttl_hours: "TTL de caché (horas)",
  egress_bind: "Interfaz de salida",
  http_port: "Puerto HTTP",
  data_dir: "Directorio de datos",
};
const NUMERIC = new Set(["cache_max_gb", "cache_ttl_hours", "http_port"]);

async function viewSettings() {
  setTab("settings");
  const s = await api("/api/settings");
  const inputs = {};
  const form = el("form", { class: "form", onsubmit: async (e) => {
    e.preventDefault();
    const body = {};
    for (const f of [...HOT_FIELDS, ...STATIC_FIELDS]) {
      const raw = inputs[f].value.trim();
      if (raw === "") continue;
      body[f] = NUMERIC.has(f) ? Number(raw) : raw;
    }
    try {
      const r = await api("/api/settings", {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      });
      const applied = (r.applied || []).length ? `Aplicado: ${r.applied.join(", ")}. ` : "";
      const restart = (r.requires_restart || []).length
        ? `Requiere reinicio: ${r.requires_restart.map((k) => LABELS[k] || k).join(", ")}.`
        : "";
      showBanner(applied + restart || "Guardado");
      route();
    } catch (err) { showBanner(err.message); }
  }});
  for (const f of [...HOT_FIELDS, ...STATIC_FIELDS]) {
    const input = el("input", { value: s[f] ?? "", type: NUMERIC.has(f) ? "number" : "text" });
    inputs[f] = input;
    const label = el("label", { class: "field" }, el("span", {}, LABELS[f] || f), input);
    if (STATIC_FIELDS.includes(f)) label.append(el("small", { class: "muted" }, "requiere reinicio"));
    form.append(label);
  }
  form.append(el("button", { class: "btn primary", type: "submit" }, "Guardar"));
  app.replaceChildren(el("h1", {}, "Ajustes"), form);
}

window.addEventListener("hashchange", route);
route();
```

- [ ] **Step 4: Correr y ver pasar**

Run: `cd ~/Proyectos/PiStreaming && cargo test -p pistreaming-api --test spa_assets`

Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/api/assets/app.js crates/api/tests/spa_assets.rs
git commit -m "feat(api): router por hash y vistas de la SPA"
```

---

## Verificación final

Ejecutar desde `~/Proyectos/PiStreaming` (no es una tarea con commit; es el gate de cierre):

- [ ] `cargo test -p pistreaming-api -p pistreaming-store` → todo verde (store: CRUD/`keep`/`link_or_copy`/`library_info_hashes`; api: settings, assets, library, patch addons, y los tests previos).
- [ ] `cargo test -p pistreaming-server` → verde (eviction excluye biblioteca y lee settings al vuelo).
- [ ] `cargo clippy` → sin warnings nuevas.
- [ ] **No** ejecutar `cargo fmt --check` (roto preexistente por deuda anterior).
- [ ] Manual en el Pi (spec §8): `http://192.168.100.100:8000/` desde la LAN → Buscar → Detalle → Reproducir → Guardar → Biblioteca → Reproducir local → Addons (alta/toggle/borrar) → Ajustes (editar y ver «requiere reinicio»).

## Cobertura del spec (checklist de trazabilidad)

- §3.1-3.3 assets + `rust-embed` + hash-routing → Tasks 5, 8, 9.
- §4.1 tabla `library` → Task 1. §4.2 `keep` (hardlink/copia + 409) → Tasks 1, 6. §4.3-4.4 limpieza y CRUD → Tasks 1, 3.
- §5.1 biblioteca y `/library/:id/stream` → Task 6. §5.2 `PATCH /api/addons/:url` → Task 7. §5.3 settings → Task 2. §5.4 estáticos → Task 5. §5.5 sin cambios (`/stream`, `/api/progress`, `POST /api/play`) → intactos.
- §6 vistas y reglas de UI (tema, reproductor como vista propia, progreso ~10 s, estados) → Tasks 8, 9.
- §7 bordes (409 incompleta, 404, 416 vía `ranged_response`, 400 con campo, fallback de copia) → Tasks 1, 2, 6.
- §8 verificación → Verificación final.
