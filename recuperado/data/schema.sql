CREATE TABLE addons (
  url           TEXT PRIMARY KEY,
  manifest_json TEXT NOT NULL,
  name          TEXT NOT NULL,
  enabled       INTEGER NOT NULL DEFAULT 1,
  position      INTEGER NOT NULL DEFAULT 0,
  added_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE library (
  id          TEXT PRIMARY KEY,
  kind        TEXT NOT NULL,
  title       TEXT NOT NULL,
  file_path   TEXT NOT NULL,
  size_bytes  INTEGER NOT NULL,
  info_hash   TEXT,
  created_at  INTEGER NOT NULL
);

CREATE TABLE metadata_cache (
  key        TEXT PRIMARY KEY,
  json       TEXT NOT NULL,
  expires_at TEXT NOT NULL
);

CREATE TABLE play_sessions (
  id             TEXT PRIMARY KEY,
  info_hash      TEXT NOT NULL,
  file_id        INTEGER NOT NULL,
  magnet         TEXT,
  plan_json      TEXT NOT NULL,
  kind           TEXT,
  meta_id        TEXT,
  title          TEXT,
  progress_key   TEXT,
  transcode_done INTEGER NOT NULL DEFAULT 0,
  created_at     INTEGER NOT NULL
);

CREATE TABLE progress (
  id         TEXT PRIMARY KEY,
  position   REAL NOT NULL DEFAULT 0,
  duration   REAL,
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
