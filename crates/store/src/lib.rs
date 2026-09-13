//! Persistencia SQLite de PiStreaming.

use pistreaming_core::error::{CoreError, CoreResult};
use pistreaming_core::manifest::Manifest;
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
}
