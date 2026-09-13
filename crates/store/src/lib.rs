//! Persistencia SQLite de PiStreaming.

use pistreaming_core::error::{CoreError, CoreResult};
use pistreaming_core::manifest::Manifest;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;
use std::time;

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
"#;

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

    pub fn remove_addon(&self, url: &str) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM addons WHERE url = ?1", params![url])
            .map_err(|e| CoreError::Db(e.to_string()))?;
        Ok(())
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
}
