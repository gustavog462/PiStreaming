//! Persistencia SQLite de PiStreaming.

use pistreaming_core::error::{CoreError, CoreResult};
use pistreaming_core::manifest::Manifest;
use pistreaming_core::normalize_url;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddonRow {
    pub url: String,
    pub manifest: Manifest,
    pub name: String,
    pub enabled: bool,
    pub position: i64,
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> CoreResult<Self> {
        let conn = Connection::open(path).map_err(|e| CoreError::Db(e.to_string()))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| CoreError::Db(e.to_string()))?;
        conn.execute_batch(MIGRATIONS)
            .map_err(|e| CoreError::Db(e.to_string()))?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn add_addon(&self, url: &str, manifest: &Manifest) -> CoreResult<()> {
        let url = normalize_url(url);
        let json = serde_json::to_string(manifest)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO addons (url, manifest_json, name) VALUES (?1, ?2, ?3)
             ON CONFLICT(url) DO UPDATE SET manifest_json = excluded.manifest_json,
                                            name = excluded.name",
            params![url, json, manifest.name],
        )
        .map_err(|e| CoreError::Db(e.to_string()))?;
        Ok(())
    }

    pub fn list_addons(&self) -> CoreResult<Vec<AddonRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT url, manifest_json, name, enabled, position FROM addons ORDER BY position, added_at")
            .map_err(|e| CoreError::Db(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                let url: String = row.get(0)?;
                let manifest_json: String = row.get(1)?;
                let name: String = row.get(2)?;
                let enabled: i64 = row.get(3)?;
                let position: i64 = row.get(4)?;
                Ok((url, manifest_json, name, enabled, position))
            })
            .map_err(|e| CoreError::Db(e.to_string()))?;

        let mut out = Vec::new();
        for r in rows {
            let (url, manifest_json, name, enabled, position) =
                r.map_err(|e| CoreError::Db(e.to_string()))?;
            let manifest: Manifest = serde_json::from_str(&manifest_json)?;
            out.push(AddonRow { url, manifest, name, enabled: enabled != 0, position });
        }
        Ok(out)
    }

    pub fn set_addon_enabled(&self, url: &str, enabled: bool) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE addons SET enabled = ?1 WHERE url = ?2",
            params![enabled as i64, url],
        )
        .map_err(|e| CoreError::Db(e.to_string()))?;
        Ok(())
    }

    /// Borra el addon. Devuelve `true` si había una fila (y se borró).
    pub fn remove_addon(&self, url: &str) -> CoreResult<bool> {
        let url = normalize_url(url);
        let conn = self.conn.lock().unwrap();
        let changes = conn
            .execute("DELETE FROM addons WHERE url = ?1", params![url])
            .map_err(|e| CoreError::Db(e.to_string()))?;
        Ok(changes > 0)
    }

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

    pub fn get_setting(&self, key: &str) -> CoreResult<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT value FROM settings WHERE key = ?1")
            .map_err(|e| CoreError::Db(e.to_string()))?;
        let mut rows = stmt.query(params![key]).map_err(|e| CoreError::Db(e.to_string()))?;
        if let Some(row) = rows.next().map_err(|e| CoreError::Db(e.to_string()))? {
            Ok(Some(row.get(0).map_err(|e| CoreError::Db(e.to_string()))?))
        } else {
            Ok(None)
        }
    }

    pub fn set_setting(&self, key: &str, value: &str) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(|e| CoreError::Db(e.to_string()))?;
        Ok(())
    }

    pub fn save_progress(&self, id: &str, position: f64, duration: Option<f64>) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO progress (id, position, duration, updated_at) VALUES (?1, ?2, ?3, datetime('now'))
             ON CONFLICT(id) DO UPDATE SET position = excluded.position,
                                           duration = excluded.duration,
                                           updated_at = datetime('now')",
            params![id, position, duration],
        )
        .map_err(|e| CoreError::Db(e.to_string()))?;
        Ok(())
    }

    pub fn get_progress(&self, id: &str) -> CoreResult<Option<(f64, Option<f64>)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT position, duration FROM progress WHERE id = ?1")
            .map_err(|e| CoreError::Db(e.to_string()))?;
        let mut rows = stmt.query(params![id]).map_err(|e| CoreError::Db(e.to_string()))?;
        if let Some(row) = rows.next().map_err(|e| CoreError::Db(e.to_string()))? {
            let pos: f64 = row.get(0).map_err(|e| CoreError::Db(e.to_string()))?;
            let dur: Option<f64> = row.get(1).map_err(|e| CoreError::Db(e.to_string()))?;
            Ok(Some((pos, dur)))
        } else {
            Ok(None)
        }
    }

    pub fn put_metadata_cache(&self, key: &str, json: &str, ttl_secs: i64) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO metadata_cache (key, json, expires_at)
             VALUES (?1, ?2, datetime('now', ?3))
             ON CONFLICT(key) DO UPDATE SET json = excluded.json,
                                            expires_at = excluded.expires_at",
            params![key, json, format!("{ttl_secs} seconds")],
        )
        .map_err(|e| CoreError::Db(e.to_string()))?;
        Ok(())
    }

    pub fn get_metadata_cache(&self, key: &str) -> CoreResult<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT json FROM metadata_cache WHERE key = ?1 AND expires_at > datetime('now')")
            .map_err(|e| CoreError::Db(e.to_string()))?;
        let mut rows = stmt.query(params![key]).map_err(|e| CoreError::Db(e.to_string()))?;
        if let Some(row) = rows.next().map_err(|e| CoreError::Db(e.to_string()))? {
            Ok(Some(row.get(0).map_err(|e| CoreError::Db(e.to_string()))?))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pistreaming_core::manifest::Manifest;

    fn manifest(id: &str, name: &str) -> Manifest {
        Manifest {
            id: id.into(),
            version: "1.0.0".into(),
            name: name.into(),
            description: None,
            resources: vec![],
            types: vec![],
            catalogs: vec![],
            id_prefixes: None,
        }
    }

    #[test]
    fn addon_crud_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();

        store
            .add_addon("https://a.test", &manifest("org.a", "Addon A"))
            .unwrap();
        store
            .add_addon("https://b.test", &manifest("org.b", "Addon B"))
            .unwrap();

        let rows = store.list_addons().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|r| r.name == "Addon A"));

        store.set_addon_enabled("https://a.test", false).unwrap();
        let rows = store.list_addons().unwrap();
        let a = rows.iter().find(|r| r.url == "https://a.test").unwrap();
        assert!(!a.enabled);

        store.remove_addon("https://b.test").unwrap();
        assert_eq!(store.list_addons().unwrap().len(), 1);
    }

    #[test]
    fn adding_same_url_upserts() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();
        store.add_addon("https://a.test", &manifest("org.a", "Old")).unwrap();
        store.add_addon("https://a.test", &manifest("org.a", "New")).unwrap();
        let rows = store.list_addons().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "New");
    }

    #[test]
    fn settings_roundtrip_and_default() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();
        assert_eq!(store.get_setting("egress_bind").unwrap(), None);
        store.set_setting("egress_bind", "eth0").unwrap();
        assert_eq!(store.get_setting("egress_bind").unwrap().as_deref(), Some("eth0"));
        store.set_setting("egress_bind", "wg0").unwrap();
        assert_eq!(store.get_setting("egress_bind").unwrap().as_deref(), Some("wg0"));
    }

    #[test]
    fn progress_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();
        store.save_progress("tt1160419", 1234.5, Some(9300.0)).unwrap();
        let p = store.get_progress("tt1160419").unwrap().unwrap();
        assert_eq!(p.0, 1234.5);
        assert_eq!(p.1, Some(9300.0));
    }

    #[test]
    fn metadata_cache_hits_then_expires() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();
        store.put_metadata_cache("meta:tt100", r#"{"a":1}"#, 3600).unwrap();
        assert_eq!(store.get_metadata_cache("meta:tt100").unwrap().as_deref(), Some(r#"{"a":1}"#));
        store.put_metadata_cache("meta:tt101", r#"{"b":2}"#, -1).unwrap();
        assert_eq!(store.get_metadata_cache("meta:tt101").unwrap(), None);
    }

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
}
