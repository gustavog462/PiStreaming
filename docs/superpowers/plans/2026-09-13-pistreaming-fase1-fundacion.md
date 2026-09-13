# PiStreaming — Fase 1: Fundación + motor de addons — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use omo-subagent-driven-development (recommended) or omo-dispatching-parallel-agents to implement this plan task-by-task. Each task should specify a `category` (quick/deep/ultrabrain/visual-engineering) and `load_skills` for oh-my-opencode's task() tool. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Levantar un server Rust que carga addons del protocolo de Stremio, federar búsquedas y devolver meta/streams por una API HTTP, con tests verdes.

**Architecture:** Workspace Rust de crates de un solo propósito. `core` = modelos y helpers puros (sin I/O). `store` = SQLite (rusqlite). `addons` = cliente HTTP del protocolo de addons (reqwest) + `AddonManager` que federa. `api` = router axum. `server` = binario: config + wiring. Los crates `torrent`/`media` se crean vacíos aquí y se implementan en la Fase 2.

**Tech Stack:** Rust 2021 · tokio · axum 0.7 · reqwest (rustls) · serde/serde_json · rusqlite (bundled) · tracing · wiremock + tempfile (tests).

**Nota de diseño (YAGNI):** el spec mencionaba traits para mockear. Se omiten en Fase 1 porque las costuras de test ya existen de forma externa: `wiremock` simula addons HTTP y `tempfile` aísla SQLite. No hay indirección sin necesidad.

**Verificación de la fase:** `cargo test --workspace` verde + `cargo run -p pistreaming-server` arranca y responde `GET /api/health` y `GET /api/search?query=dune` contra un addon mock.

---

## Estructura de archivos

```
PiStreaming/
├── Cargo.toml                                  # workspace + deps compartidas
├── .gitignore
├── rust-toolchain.toml                         # pin de toolchain
├── crates/
│   ├── core/
│   │   ├── Cargo.toml
│   │   └── src/lib.rs                          # re-exports
│   │       ├── manifest.rs                     # Manifest, Resource, CatalogEntry, ExtraProp
│   │       ├── catalog.rs                      # CatalogEntry ya en manifest; aquí CatalogRequest
│   │       ├── meta.rs                         # MetaItem, MetaDetail
│   │       ├── stream.rs                       # Stream, Quality, parse/rank
│   │       ├── playback.rs                     # PlaybackPlan, PlaybackRoute
│   │       └── error.rs                        # CoreError, CoreResult
│   ├── store/
│   │   ├── Cargo.toml
│   │   └── src/lib.rs                          # Store (rusqlite), MIGRATIONS
│   ├── addons/
│   │   ├── Cargo.toml
│   │   └── src/lib.rs                          # AddonClient, AddonManager, AddonRef
│   ├── api/
│   │   ├── Cargo.toml
│   │   └── src/lib.rs                          # AppState, router(), handlers
│   ├── server/
│   │   ├── Cargo.toml
│   │   └── src/main.rs                         # Config, boot
│   ├── torrent/  (vacío en Fase 1)
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   └── media/    (vacío en Fase 1)
│       ├── Cargo.toml
│       └── src/lib.rs
```

---

## Task 1: Scaffold del workspace y crates

**Files:**
- Create: `Cargo.toml`
- Create: `.gitignore`
- Create: `rust-toolchain.toml`
- Create: `crates/core/Cargo.toml`, `crates/core/src/lib.rs`
- Create: `crates/store/Cargo.toml`, `crates/store/src/lib.rs`
- Create: `crates/addons/Cargo.toml`, `crates/addons/src/lib.rs`
- Create: `crates/api/Cargo.toml`, `crates/api/src/lib.rs`
- Create: `crates/server/Cargo.toml`, `crates/server/src/main.rs`
- Create: `crates/torrent/Cargo.toml`, `crates/torrent/src/lib.rs`
- Create: `crates/media/Cargo.toml`, `crates/media/src/lib.rs`

- [ ] **Step 1: Escribir el `Cargo.toml` raíz**

```toml
[workspace]
resolver = "2"
members = [
  "crates/core",
  "crates/store",
  "crates/addons",
  "crates/api",
  "crates/server",
  "crates/torrent",
  "crates/media",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"

[workspace.dependencies]
anyhow = "1"
thiserror = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "time", "signal"] }
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
axum = "0.7"
tower = "0.4"
tower-http = { version = "0.6", features = ["trace", "cors"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
rusqlite = { version = "0.32", features = ["bundled"] }
toml = "0.8"
futures = "0.3"
regex = "1"
once_cell = "1"

[dev-dependencies]
tempfile = "3"
wiremock = "0.6"
```

- [ ] **Step 2: Escribir `.gitignore`**

```gitignore
/target
**/*.rs.bk
*.db
*.db-wal
*.db-shm
.env
```

- [ ] **Step 3: Escribir `rust-toolchain.toml`**

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

- [ ] **Step 4: Escribir los `Cargo.toml` de cada crate**

`crates/core/Cargo.toml`:
```toml
[package]
name = "pistreaming-core"
version.workspace = true
edition.workspace = true

[dependencies]
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
regex.workspace = true
once_cell.workspace = true
```

`crates/store/Cargo.toml`:
```toml
[package]
name = "pistreaming-store"
version.workspace = true
edition.workspace = true

[dependencies]
pistreaming-core = { path = "../core" }
serde.workspace = true
serde_json.workspace = true
rusqlite.workspace = true
thiserror.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

`crates/addons/Cargo.toml`:
```toml
[package]
name = "pistreaming-addons"
version.workspace = true
edition.workspace = true

[dependencies]
pistreaming-core = { path = "../core" }
reqwest.workspace = true
serde.workspace = true
serde_json.workspace = true
tokio.workspace = true
futures.workspace = true
tracing.workspace = true
thiserror.workspace = true

[dev-dependencies]
wiremock.workspace = true
```

`crates/api/Cargo.toml`:
```toml
[package]
name = "pistreaming-api"
version.workspace = true
edition.workspace = true

[dependencies]
pistreaming-core = { path = "../core" }
pistreaming-store = { path = "../store" }
pistreaming-addons = { path = "../addons" }
axum.workspace = true
tower.workspace = true
tower-http.workspace = true
serde.workspace = true
serde_json.workspace = true
tokio.workspace = true
tracing.workspace = true

[dev-dependencies]
tower = { workspace = true, features = ["util"] }
http-body-util = "0.1"
tempfile.workspace = true
```

`crates/server/Cargo.toml`:
```toml
[package]
name = "pistreaming-server"
version.workspace = true
edition.workspace = true

[[bin]]
name = "pistreaming"
path = "src/main.rs"

[dependencies]
pistreaming-core = { path = "../core" }
pistreaming-store = { path = "../store" }
pistreaming-addons = { path = "../addons" }
pistreaming-api = { path = "../api" }
tokio.workspace = true
axum.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
serde.workspace = true
toml.workspace = true
anyhow.workspace = true
reqwest.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

`crates/torrent/Cargo.toml` y `crates/media/Cargo.toml` (idénticos salvo el nombre):
```toml
[package]
name = "pistreaming-torrent"
version.workspace = true
edition.workspace = true

[dependencies]
pistreaming-core = { path = "../core" }
```
(duplicar con `name = "pistreaming-media"`)

- [ ] **Step 5: Escribir stubs mínimos**

`crates/core/src/lib.rs`:
```rust
//! Modelos y helpers puros del protocolo de addons de Stremio. Sin I/O.
pub mod error;

pub use error::{CoreError, CoreResult};
```

`crates/core/src/error.rs`:
```rust
use thiserror::Error;

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("http error: {0}")]
    Http(String),
    #[error("json error: {0}")]
    Json(String),
    #[error("db error: {0}")]
    Db(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("addon error: {0}")]
    Addon(String),
    #[error("{0}")]
    Other(String),
}

impl From<serde_json::Error> for CoreError {
    fn from(e: serde_json::Error) -> Self {
        CoreError::Json(e.to_string())
    }
}
```

`crates/store/src/lib.rs`:
```rust
//! Persistencia SQLite.
```

`crates/addons/src/lib.rs`:
```rust
//! Cliente del protocolo de addons de Stremio.
```

`crates/api/src/lib.rs`:
```rust
//! API HTTP (axum).
```

`crates/server/src/main.rs`:
```rust
fn main() {
    println!("pistreaming booting");
}
```

`crates/torrent/src/lib.rs` y `crates/media/src/lib.rs`:
```rust
//! (Fase 2)
```

- [ ] **Step 6: Compilar**

Run: `cargo build --workspace`
Expected: compila sin errores (los crates vacíos avisan "unused" pero no fallan).

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml .gitignore rust-toolchain.toml crates/
git commit -m "chore: scaffold workspace Rust con crates de PiStreaming"
```

---

## Task 2: `core` — modelos de Manifest + parseo (TDD)

**Files:**
- Create: `crates/core/src/manifest.rs`
- Modify: `crates/core/src/lib.rs`
- Test: `crates/core/src/manifest.rs` (módulo `#[cfg(test)]`)

- [ ] **Step 1: Escribir el test que falla**

En `crates/core/src/manifest.rs`, al final:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_string_resources_and_catalogs() {
        let raw = r#"{
            "id": "org.torrentio",
            "version": "0.0.1",
            "name": "Torrentio",
            "resources": ["catalog", "meta", "stream"],
            "types": ["movie", "series"],
            "catalogs": [{"type": "movie", "id": "top", "name": "Top"}]
        }"#;
        let m: Manifest = serde_json::from_str(raw).unwrap();
        assert_eq!(m.id, "org.torrentio");
        assert_eq!(m.resources.len(), 3);
        assert_eq!(m.resources[0], Resource::Simple("catalog".into()));
        assert_eq!(m.catalogs[0].kind, "movie");
        assert_eq!(m.catalogs[0].name.as_deref(), Some("Top"));
    }

    #[test]
    fn parses_full_resource_and_id_prefixes() {
        let raw = r#"{
            "id": "org.cinemeta", "version": "3.0.0", "name": "Cinemeta",
            "resources": [{"name": "catalog", "types": ["movie"], "idPrefixes": ["tt"]}],
            "types": ["movie"],
            "idPrefixes": ["tt"],
            "catalogs": [{"type": "movie", "id": "top", "extra": [{"name": "search", "isRequired": true}]}]
        }"#;
        let m: Manifest = serde_json::from_str(raw).unwrap();
        match &m.resources[0] {
            Resource::Full { name, types, id_prefixes } => {
                assert_eq!(name, "catalog");
                assert_eq!(types, &vec!["movie".to_string()]);
                assert_eq!(id_prefixes.as_deref(), Some(&["tt".to_string()][..]));
            }
            _ => panic!("esperaba Resource::Full"),
        }
        assert!(m.catalogs[0].extra[0].is_required);
    }
}
```

- [ ] **Step 2: Correr el test para verlo fallar**

Run: `cargo test -p pistreaming-core manifest -- --nocapture`
Expected: FAIL — `cannot find type Manifest` / módulo inexistente.

- [ ] **Step 3: Implementar los modelos**

Escribir en `crates/core/src/manifest.rs` (arriba de los tests):
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub resources: Vec<Resource>,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub catalogs: Vec<CatalogEntry>,
    #[serde(default, rename = "idPrefixes")]
    pub id_prefixes: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Resource {
    Simple(String),
    Full {
        name: String,
        #[serde(default)]
        types: Vec<String>,
        #[serde(default, rename = "idPrefixes")]
        id_prefixes: Option<Vec<String>>,
    },
}

impl Resource {
    pub fn name(&self) -> &str {
        match self {
            Resource::Simple(n) => n,
            Resource::Full { name, .. } => name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogEntry {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub extra: Vec<ExtraProp>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtraProp {
    pub name: String,
    #[serde(default, rename = "isRequired")]
    pub is_required: bool,
    #[serde(default)]
    pub options: Vec<String>,
}

impl Manifest {
    /// ¿El addon declara el recurso dado? (p.ej. "stream")
    pub fn supports(&self, resource: &str) -> bool {
        self.resources.iter().any(|r| r.name() == resource)
    }

    /// Catálogos que sirven para búsqueda por texto.
    pub fn search_catalogs(&self) -> Vec<&CatalogEntry> {
        self.catalogs
            .iter()
            .filter(|c| c.extra.iter().any(|e| e.name == "search"))
            .collect()
    }
}
```

Actualizar `crates/core/src/lib.rs`:
```rust
pub mod error;
pub mod manifest;

pub use error::{CoreError, CoreResult};
```

- [ ] **Step 4: Correr el test para verlo pasar**

Run: `cargo test -p pistreaming-core manifest -- --nocapture`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(core): modelos de Manifest del protocolo de addons"
```

---

## Task 3: `core` — models de catalog/meta/stream (TDD)

**Files:**
- Create: `crates/core/src/catalog.rs`, `crates/core/src/meta.rs`, `crates/core/src/stream.rs`
- Modify: `crates/core/src/lib.rs`

- [ ] **Step 1: Escribir los tests que fallan**

`crates/core/src/catalog.rs`:
```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogRequest {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    #[serde(default)]
    pub extra: HashMap<String, String>,
}

impl CatalogRequest {
    pub fn search(kind: &str, id: &str, query: &str) -> Self {
        let mut extra = HashMap::new();
        extra.insert("search".to_string(), query.to_string());
        Self { kind: kind.to_string(), id: id.to_string(), extra }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_request_carries_query() {
        let req = CatalogRequest::search("movie", "top", "dune");
        assert_eq!(req.kind, "movie");
        assert_eq!(req.extra.get("search").unwrap(), "dune");
    }
}
```

`crates/core/src/meta.rs`:
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaItem {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub poster: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetaDetail {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub poster: Option<String>,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "releaseInfo")]
    pub release_info: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub runtime: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_meta_item_and_detail() {
        let item: MetaItem = serde_json::from_str(
            r#"{"id":"tt1160419","type":"movie","name":"Dune","poster":"p.jpg"}"#,
        )
        .unwrap();
        assert_eq!(item.kind, "movie");
        let detail: MetaDetail = serde_json::from_str(
            r#"{"id":"tt1160419","type":"movie","name":"Dune","genres":["Sci-Fi"],"releaseInfo":"2021"}"#,
        )
        .unwrap();
        assert_eq!(detail.genres, vec!["Sci-Fi".to_string()]);
        assert_eq!(detail.release_info.as_deref(), Some("2021"));
    }
}
```

`crates/core/src/stream.rs`:
```rust
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

static QUALITY_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)(2160p|4k|1080p|720p|480p)").unwrap());
static SEEDS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"👤\s*(\d+)").unwrap());

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Quality {
    Unknown = 0,
    K480 = 1,
    K720 = 2,
    K1080 = 3,
    K4 = 4,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stream {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default, rename = "infoHash")]
    pub info_hash: Option<String>,
    #[serde(default)]
    pub fileIdx: Option<u32>,
    #[serde(default)]
    pub sources: Vec<String>,
    /// Addon que lo aportó. Se rellena al federar (no viene del addon).
    #[serde(skip)]
    pub source_addon: Option<String>,
}

impl Stream {
    pub fn text(&self) -> String {
        format!(
            "{} {}",
            self.name.clone().unwrap_or_default(),
            self.title.clone().unwrap_or_default()
        )
    }

    pub fn quality(&self) -> Quality {
        let hay = self.text().to_lowercase();
        if hay.contains("2160p") || hay.contains("4k") {
            Quality::K4
        } else if hay.contains("1080p") {
            Quality::K1080
        } else if hay.contains("720p") {
            Quality::K720
        } else if hay.contains("480p") {
            Quality::K480
        } else {
            Quality::Unknown
        }
    }

    pub fn seeds(&self) -> Option<u32> {
        SEEDS_RE
            .captures(&self.text())
            .and_then(|c| c.get(1))
            .and_then(|m| m.as_str().parse().ok())
    }

    pub fn is_playable(&self) -> bool {
        (self.info_hash.is_some() || self.url.is_some())
            && self.seeds().map(|s| s > 0).unwrap_or(self.url.is_some())
    }
}

/// Ordena: calidad desc, luego seeds desc, luego los reproducibles primero.
pub fn rank_streams(streams: &mut [Stream]) {
    streams.sort_by(|a, b| {
        b.quality()
            .cmp(&a.quality())
            .then_with(|| b.seeds().unwrap_or(0).cmp(&a.seeds().unwrap_or(0)))
            .then_with(|| b.is_playable().cmp(&a.is_playable()))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(name: &str) -> Stream {
        Stream {
            name: Some(name.into()),
            title: None,
            url: None,
            info_hash: Some("abc".into()),
            fileIdx: None,
            sources: vec![],
            source_addon: None,
        }
    }

    #[test]
    fn infers_quality() {
        assert_eq!(s("Movie 2160p HDR").quality(), Quality::K4);
        assert_eq!(s("Movie 1080p").quality(), Quality::K1080);
        assert_eq!(s("Movie 720p").quality(), Quality::K720);
        assert_eq!(s("Movie").quality(), Quality::Unknown);
    }

    #[test]
    fn parses_seeds_emoji() {
        let mut st = s("Torrent 🧲");
        st.title = Some("👤 421 💾 12 GB".into());
        assert_eq!(st.seeds(), Some(421));
    }

    #[test]
    fn ranks_4k_above_1080p() {
        let mut v = vec![s("Movie 1080p"), s("Movie 2160p"), s("Movie 720p")];
        rank_streams(&mut v);
        assert_eq!(v[0].quality(), Quality::K4);
        assert_eq!(v[1].quality(), Quality::K1080);
    }
}
```

- [ ] **Step 2: Correr los tests para verlos fallar**

Run: `cargo test -p pistreaming-core -- --nocapture`
Expected: FAIL — módulos `catalog`/`meta`/`stream` no declarados.

- [ ] **Step 3: Declarar los módulos**

`crates/core/src/lib.rs`:
```rust
pub mod catalog;
pub mod error;
pub mod manifest;
pub mod meta;
pub mod stream;

pub use error::{CoreError, CoreResult};
```

- [ ] **Step 4: Correr los tests para verlos pasar**

Run: `cargo test -p pistreaming-core -- --nocapture`
Expected: PASS (todos los tests de core).

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(core): modelos catalog/meta/stream + ranking de calidad"
```

---

## Task 4: `core` — PlaybackPlan (TDD)

**Files:**
- Create: `crates/core/src/playback.rs`
- Modify: `crates/core/src/lib.rs`

- [ ] **Step 1: Escribir el test que falla**

`crates/core/src/playback.rs`:
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackRoute {
    /// El browser toma el archivo tal cual (o remux trivial).
    Direct,
    /// Remux MKV→fMP4 (cambio de contenedor, sin recodificar video).
    Remux,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybackPlan {
    pub session: String,
    pub route: PlaybackRoute,
    /// URL que consume el <video>.
    pub playback_url: String,
    /// URL cruda del .mkv para mpv/VLC cuando el browser no decodifica.
    pub raw_url: Option<String>,
    pub browser_may_fail: bool,
    pub needs_recode_audio: bool,
    pub video_codec: String,
    pub audio_codec: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_route_lowercase() {
        let p = PlaybackPlan {
            session: "s1".into(),
            route: PlaybackRoute::Remux,
            playback_url: "/stream/s1".into(),
            raw_url: Some("/raw/s1.mkv".into()),
            browser_may_fail: true,
            needs_recode_audio: false,
            video_codec: "hevc".into(),
            audio_codec: Some("eac3".into()),
        };
        let j = serde_json::to_string(&p).unwrap();
        assert!(j.contains("\"route\":\"remux\""));
        assert!(j.contains("\"video_codec\":\"hevc\""));
    }
}
```

- [ ] **Step 2: Correr el test para verlo fallar**

Run: `cargo test -p pistreaming-core playback -- --nocapture`
Expected: FAIL — módulo inexistente.

- [ ] **Step 3: Declarar el módulo**

`crates/core/src/lib.rs` (agregar `pub mod playback;`).

- [ ] **Step 4: Correr el test para verlo pasar**

Run: `cargo test -p pistreaming-core playback -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/core
git commit -m "feat(core): PlaybackPlan y PlaybackRoute"
```

---

## Task 5: `store` — apertura, migraciones y CRUD de addons (TDD)

**Files:**
- Modify: `crates/store/src/lib.rs`
- Test: `crates/store/src/lib.rs` (módulo `#[cfg(test)]`)

- [ ] **Step 1: Escribir el test que falla**

`crates/store/src/lib.rs`, al final:
```rust
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
```

- [ ] **Step 2: Correr los tests para verlos fallar**

Run: `cargo test -p pistreaming-store -- --nocapture`
Expected: FAIL — `Store` no existe.

- [ ] **Step 3: Implementar `Store`**

Escribir en `crates/store/src/lib.rs` (arriba de los tests):
```rust
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
```

- [ ] **Step 4: Correr los tests para verlos pasar**

Run: `cargo test -p pistreaming-store -- --nocapture`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/store
git commit -m "feat(store): apertura SQLite + CRUD de addons"
```

---

## Task 6: `store` — settings, progress, metadata_cache (TDD)

**Files:**
- Modify: `crates/store/src/lib.rs`

- [ ] **Step 1: Escribir los tests que fallan**

Agregar dentro de `mod tests` de `crates/store/src/lib.rs`:
```rust
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
```

- [ ] **Step 2: Correr los tests para verlos fallar**

Run: `cargo test -p pistreaming-store -- --nocapture`
Expected: FAIL — métodos `get_setting`/`set_setting`/`save_progress`/`get_progress`/`put_metadata_cache`/`get_metadata_cache` inexistentes.

- [ ] **Step 3: Implementar los métodos**

Agregar al `impl Store` (antes del cierre, línea del `impl`), y agregar `std::time` al final del bloque de `use`:
```rust
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
```

- [ ] **Step 4: Correr los tests para verlos pasar**

Run: `cargo test -p pistreaming-store -- --nocapture`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/store
git commit -m "feat(store): settings, progress y metadata_cache"
```

---

## Task 7: `addons` — `AddonClient` (TDD con wiremock)

**Files:**
- Modify: `crates/addons/src/lib.rs`
- Test: `crates/addons/src/lib.rs` (módulo `#[cfg(test)]`)

- [ ] **Step 1: Escribir el test que falla**

`crates/addons/src/lib.rs`, al final:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn fetch_manifest_ok() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/manifest.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "org.test", "version": "1.0.0", "name": "Test",
                "resources": ["catalog", "meta", "stream"],
                "types": ["movie"],
                "catalogs": [{"type": "movie", "id": "top", "extra": [{"name": "search"}]}]
            })))
            .mount(&server)
            .await;

        let client = AddonClient::new(reqwest::Client::new());
        let m = client.fetch_manifest(&server.uri()).await.unwrap();
        assert_eq!(m.id, "org.test");
        assert!(m.supports("stream"));
        assert_eq!(m.search_catalogs().len(), 1);
    }

    #[tokio::test]
    async fn fetch_manifest_maps_500_to_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/manifest.json"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = AddonClient::new(reqwest::Client::new());
        assert!(client.fetch_manifest(&server.uri()).await.is_err());
    }

    #[tokio::test]
    async fn streams_parses_payload() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/stream/movie/tt1160419.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "streams": [
                    {"name": "Torrentio", "title": "2160p 👤 300", "infoHash": "abc123"},
                    {"name": "Torrentio", "title": "1080p 👤 900", "infoHash": "def456"}
                ]
            })))
            .mount(&server)
            .await;

        let client = AddonClient::new(reqwest::Client::new());
        let streams = client.streams(&server.uri(), "movie", "tt1160419").await.unwrap();
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0].info_hash.as_deref(), Some("abc123"));
    }
}
```

- [ ] **Step 2: Correr los tests para verlos fallar**

Run: `cargo test -p pistreaming-addons -- --nocapture`
Expected: FAIL — `AddonClient` no existe.

- [ ] **Step 3: Implementar `AddonClient`**

Escribir en `crates/addons/src/lib.rs` (arriba de los tests):
```rust
//! Cliente HTTP del protocolo de addons de Stremio.

use pistreaming_core::catalog::CatalogRequest;
use pistreaming_core::error::{CoreError, CoreResult};
use pistreaming_core::manifest::Manifest;
use pistreaming_core::meta::{MetaDetail, MetaItem};
use pistreaming_core::stream::Stream;
use serde::Deserialize;
use std::time::Duration;

#[derive(Clone)]
pub struct AddonClient {
    http: reqwest::Client,
    timeout: Duration,
}

#[derive(Deserialize)]
struct StreamsResponse {
    #[serde(default)]
    streams: Vec<Stream>,
}

#[derive(Deserialize)]
struct CatalogResponse {
    #[serde(default)]
    metas: Vec<MetaItem>,
}

#[derive(Deserialize)]
struct MetaResponse {
    meta: MetaDetail,
}

impl AddonClient {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http, timeout: Duration::from_secs(10) }
    }

    fn base(url: &str) -> String {
        url.trim_end_matches('/').to_string()
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> CoreResult<T> {
        let resp = self
            .http
            .get(url)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| CoreError::Http(e.to_string()))?;
        let resp = resp
            .error_for_status()
            .map_err(|e| CoreError::Http(e.to_string()))?;
        resp.json::<T>()
            .await
            .map_err(|e| CoreError::Json(e.to_string()))
    }

    pub async fn fetch_manifest(&self, base: &str) -> CoreResult<Manifest> {
        let url = format!("{}/manifest.json", Self::base(base));
        self.get_json(&url).await
    }

    /// Construye la URL del catálogo según la convención de Stremio.
    pub fn catalog_url(base: &str, req: &CatalogRequest) -> String {
        let base = Self::base(base);
        if req.extra.is_empty() {
            format!("{}/catalog/{}/{}.json", base, req.kind, req.id)
        } else {
            let mut extra: Vec<String> = req
                .extra
                .iter()
                .map(|(k, v)| format!("{}={}", k, urlencoding(v)))
                .collect();
            extra.sort();
            format!("{}/catalog/{}/{}/{}.json", base, req.kind, req.id, extra.join("&"))
        }
    }

    pub async fn catalog(&self, base: &str, req: &CatalogRequest) -> CoreResult<Vec<MetaItem>> {
        let url = Self::catalog_url(base, req);
        let resp: CatalogResponse = self.get_json(&url).await?;
        Ok(resp.metas)
    }

    pub async fn meta(&self, base: &str, kind: &str, id: &str) -> CoreResult<MetaDetail> {
        let url = format!("{}/meta/{}/{}.json", Self::base(base), kind, id);
        let resp: MetaResponse = self.get_json(&url).await?;
        Ok(resp.meta)
    }

    pub async fn streams(&self, base: &str, kind: &str, id: &str) -> CoreResult<Vec<Stream>> {
        let url = format!("{}/stream/{}/{}.json", Self::base(base), kind, id);
        let resp: StreamsResponse = self.get_json(&url).await?;
        Ok(resp.streams)
    }
}

/// Codificación mínima para valores de `extra` en la URL.
fn urlencoding(s: &str) -> String {
    s.replace(' ', "%20")
        .replace('&', "%26")
        .replace('?', "%3F")
}
```

- [ ] **Step 4: Correr los tests para verlos pasar**

Run: `cargo test -p pistreaming-addons -- --nocapture`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/addons
git commit -m "feat(addons): AddonClient (manifest/catalog/meta/streams)"
```

---

## Task 8: `addons` — `AddonManager` federado (TDD con 2 addons mock)

**Files:**
- Modify: `crates/addons/src/lib.rs`

- [ ] **Step 1: Escribir el test que falla**

Agregar al `mod tests` de `crates/addons/src/lib.rs`:
```rust
    #[tokio::test]
    async fn manager_federates_and_dedupes() {
        // dos addons mock que devuelven el mismo id + uno distinto
        let a = MockServer::start().await;
        let b = MockServer::start().await;
        for server in [&a, &b] {
            Mock::given(method("GET"))
                .and(path("/manifest.json"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": format!("org.{}", server.uri().len()),
                    "version": "1.0.0", "name": "Mock",
                    "resources": ["catalog", "meta", "stream"], "types": ["movie"],
                    "catalogs": [{"type": "movie", "id": "top", "extra": [{"name": "search"}]}]
                })))
                .mount(server)
                .await;
        }
        Mock::given(method("GET"))
            .and(path("/catalog/movie/top/search=dune.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "metas": [{"id": "tt1", "type": "movie", "name": "Dune"}]
            })))
            .mount(&a)
            .await;
        Mock::given(method("GET"))
            .and(path("/catalog/movie/top/search=dune.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "metas": [
                    {"id": "tt1", "type": "movie", "name": "Dune"},
                    {"id": "tt2", "type": "movie", "name": "Dune Part Two"}
                ]
            })))
            .mount(&b)
            .await;

        let client = AddonClient::new(reqwest::Client::new());
        let mut mgr = AddonManager::new(client);
        mgr.add_from_url(&a.uri()).await.unwrap();
        mgr.add_from_url(&b.uri()).await.unwrap();

        let metas = mgr.search("movie", "dune").await;
        assert_eq!(metas.len(), 2, "dedupe por id");
        assert!(metas.iter().any(|m| m.id == "tt2"));
    }

    #[tokio::test]
    async fn manager_survives_a_dead_addon() {
        let good = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/manifest.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "org.good", "version": "1.0.0", "name": "Good",
                "resources": ["stream"], "types": ["movie"], "catalogs": []
            })))
            .mount(&good)
            .await;
        Mock::given(method("GET"))
            .and(path("/stream/movie/tt1.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "streams": [{"name": "Good", "infoHash": "abc", "title": "1080p 👤 5"}]
            })))
            .mount(&good)
            .await;

        let client = AddonClient::new(reqwest::Client::new());
        let mut mgr = AddonManager::new(client);
        mgr.add_from_url(&good.uri()).await.unwrap();
        // addon muerto: URL a un puerto cerrado
        mgr.add_from_url("http://127.0.0.1:1").await.err();

        let streams = mgr.streams("movie", "tt1").await;
        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].source_addon.as_deref(), Some("Good"));
    }
```

- [ ] **Step 2: Correr los tests para verlos fallar**

Run: `cargo test -p pistreaming-addons -- --nocapture`
Expected: FAIL — `AddonManager` no existe.

- [ ] **Step 3: Implementar `AddonManager`**

Agregar a `crates/addons/src/lib.rs` (debajo de `AddonClient`):
```rust
use futures::future::join_all;
use pistreaming_core::stream::rank_streams;
use std::collections::HashSet;
use tracing::warn;

#[derive(Clone)]
pub struct AddonRef {
    pub url: String,
    pub manifest: Manifest,
    pub enabled: bool,
}

#[derive(Clone)]
pub struct AddonManager {
    client: AddonClient,
    addons: Vec<AddonRef>,
}

impl AddonManager {
    pub fn new(client: AddonClient) -> Self {
        Self { client, addons: Vec::new() }
    }

    pub fn addons(&self) -> &[AddonRef] {
        &self.addons
    }

    /// Registra un addon fetcheando su manifest. Falla si el manifest no carga.
    pub async fn add_from_url(&mut self, url: &str) -> CoreResult<()> {
        let manifest = self.client.fetch_manifest(url).await?;
        self.addons.push(AddonRef {
            url: url.trim_end_matches('/').to_string(),
            manifest,
            enabled: true,
        });
        Ok(())
    }

    /// Carga addons desde filas de store (manifest ya cacheado).
    pub fn load(&mut self, url: &str, manifest: Manifest, enabled: bool) {
        self.addons.push(AddonRef {
            url: url.trim_end_matches('/').to_string(),
            manifest,
            enabled,
        });
    }

    pub async fn search(&self, kind: &str, query: &str) -> Vec<MetaItem> {
        let mut futs = Vec::new();
        for a in self.addons.iter().filter(|a| a.enabled) {
            for catalog in a.manifest.search_catalogs() {
                if catalog.kind != kind {
                    continue;
                }
                let req = CatalogRequest::search(&catalog.kind, &catalog.id, query);
                let base = a.url.clone();
                let client = self.client.clone();
                futs.push(async move { (a.manifest.name.clone(), client.catalog(&base, &req).await) });
            }
        }

        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for (name, res) in join_all(futs).await {
            match res {
                Ok(metas) => {
                    for m in metas {
                        if seen.insert(m.id.clone()) {
                            out.push(m);
                        }
                    }
                }
                Err(e) => warn!(addon = %name, error = %e, "catálogo falló"),
            }
        }
        out
    }

    pub async fn streams(&self, kind: &str, id: &str) -> Vec<Stream> {
        let mut futs = Vec::new();
        for a in self.addons.iter().filter(|a| a.enabled && a.manifest.supports("stream")) {
            let base = a.url.clone();
            let client = self.client.clone();
            let name = a.manifest.name.clone();
            let kind = kind.to_string();
            let id = id.to_string();
            futs.push(async move {
                let r = client.streams(&base, &kind, &id).await;
                (name, r)
            });
        }

        let mut out = Vec::new();
        for (name, res) in join_all(futs).await {
            match res {
                Ok(mut streams) => {
                    for s in streams.iter_mut() {
                        s.source_addon = Some(name.clone());
                    }
                    out.extend(streams);
                }
                Err(e) => warn!(addon = %name, error = %e, "streams falló"),
            }
        }
        rank_streams(&mut out);
        out
    }
}
```

- [ ] **Step 4: Correr los tests para verlos pasar**

Run: `cargo test -p pistreaming-addons -- --nocapture`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/addons
git commit -m "feat(addons): AddonManager federado con aislamiento de addons caídos"
```

---

## Task 9: `api` — router axum + handlers (TDD de integración)

**Files:**
- Modify: `crates/api/src/lib.rs`
- Test: `crates/api/src/lib.rs`

- [ ] **Step 1: Escribir el test que falla**

`crates/api/src/lib.rs`, al final:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use pistreaming_addons::{AddonClient, AddonManager};
    use pistreaming_store::Store;
    use std::sync::Arc;
    use tower::ServiceExt;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn test_state() -> Arc<AppState> {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/manifest.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "org.mock", "version": "1.0.0", "name": "Mock",
                "resources": ["catalog", "stream"], "types": ["movie"],
                "catalogs": [{"type": "movie", "id": "top", "extra": [{"name": "search"}]}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/catalog/movie/top/search=dune.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "metas": [{"id": "tt1", "type": "movie", "name": "Dune"}]
            })))
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();
        let mut mgr = AddonManager::new(AddonClient::new(reqwest::Client::new()));
        mgr.add_from_url(&server.uri()).await.unwrap();

        Arc::new(AppState { store, addons: mgr })
    }

    #[tokio::test]
    async fn health_ok() {
        let app = router(test_state().await);
        let resp = app
            .oneshot(Request::get("/api/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn search_returns_metas() {
        let app = router(test_state().await);
        let resp = app
            .oneshot(Request::get("/api/search?query=dune").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["metas"][0]["id"], "tt1");
    }

    #[tokio::test]
    async fn search_without_query_is_400() {
        let app = router(test_state().await);
        let resp = app
            .oneshot(Request::get("/api/search").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
```

- [ ] **Step 2: Correr los tests para verlos fallar**

Run: `cargo test -p pistreaming-api -- --nocapture`
Expected: FAIL — `AppState`/`router` inexistentes.

- [ ] **Step 3: Implementar la API**

Escribir en `crates/api/src/lib.rs` (arriba de los tests):
```rust
//! API HTTP (axum) de PiStreaming — Fase 1.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use pistreaming_addons::AddonManager;
use pistreaming_core::error::{CoreError, CoreResult};
use pistreaming_store::Store;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub struct AppState {
    pub store: Store,
    pub addons: AddonManager,
}

pub type SharedState = Arc<AppState>;

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/addons", get(list_addons).post(add_addon))
        .route("/api/addons/:url", axum::routing::delete(remove_addon))
        .route("/api/search", get(search))
        .route("/api/meta/:kind/:id", get(meta))
        .route("/api/streams/:kind/:id", get(streams))
        .with_state(state)
}

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    let body = Json(serde_json::json!({ "error": msg.into(), "code": code.as_u16() }));
    (code, body).into_response()
}

pub async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

pub async fn list_addons(State(st): State<SharedState>) -> Response {
    match st.store.list_addons() {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct AddAddonBody {
    pub url: String,
}

pub async fn add_addon(State(st): State<SharedState>, Json(body): Json<AddAddonBody>) -> Response {
    match fetch_and_store(&st, &body.url).await {
        Ok(name) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "url": body.url, "name": name })),
        )
            .into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

async fn fetch_and_store(st: &SharedState, url: &str) -> CoreResult<String> {
    let manifest = st.addons.client().fetch_manifest(url).await?;
    st.store.add_addon(url, &manifest)?;
    Ok(manifest.name)
}

pub async fn remove_addon(
    State(st): State<SharedState>,
    axum::extract::Path(url): axum::extract::Path<String>,
) -> Response {
    let url = decode_url(&url);
    match st.store.remove_addon(&url) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub query: Option<String>,
    #[serde(default = "default_kind")]
    pub kind: String,
}

fn default_kind() -> String {
    "movie".to_string()
}

pub async fn search(State(st): State<SharedState>, Query(q): Query<SearchQuery>) -> Response {
    let Some(query) = q.query.filter(|s| !s.trim().is_empty()) else {
        return err(StatusCode::BAD_REQUEST, "falta el parámetro `query`");
    };
    let metas = st.addons.search(&q.kind, &query).await;
    Json(serde_json::json!({ "metas": metas })).into_response()
}

pub async fn meta(
    State(st): State<SharedState>,
    axum::extract::Path((kind, id)): axum::extract::Path<(String, String)>,
) -> Response {
    match st.addons.meta(&kind, &id).await {
        Ok(m) => Json(m).into_response(),
        Err(e) => err(StatusCode::NOT_FOUND, e.to_string()),
    }
}

pub async fn streams(
    State(st): State<SharedState>,
    axum::extract::Path((kind, id)): axum::extract::Path<(String, String)>,
) -> Response {
    let streams = st.addons.streams(&kind, &id).await;
    Json(serde_json::json!({ "streams": streams })).into_response()
}

fn decode_url(s: &str) -> String {
    s.replace("%2F", "/").replace("%3A", ":")
}
```

Agregar al `impl AddonManager` en `crates/addons/src/lib.rs`:
```rust
    pub fn client(&self) -> &AddonClient {
        &self.client
    }

    pub async fn meta(&self, kind: &str, id: &str) -> CoreResult<MetaDetail> {
        for a in self.addons.iter().filter(|a| a.enabled && a.manifest.supports("meta")) {
            if let Ok(m) = self.client.meta(&a.url, kind, id).await {
                return Ok(m);
            }
        }
        Err(CoreError::NotFound(format!("{kind}/{id}")))
    }
```

- [ ] **Step 4: Correr los tests para verlos pasar**

Run: `cargo test -p pistreaming-api -- --nocapture`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/api crates/addons
git commit -m "feat(api): router axum con health/addons/search/meta/streams"
```

---

## Task 10: `server` — config (TOML + env) y boot (TDD)

**Files:**
- Modify: `crates/server/src/main.rs`

- [ ] **Step 1: Escribir el test que falla**

Al final de `crates/server/src/main.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::Config;

    #[test]
    fn env_overrides_defaults() {
        std::env::set_var("PISTREAMING_HTTP_PORT", "9999");
        std::env::set_var("PISTREAMING_EGRESS_BIND", "wg0");
        let cfg = Config::load(None).unwrap();
        assert_eq!(cfg.http_port, 9999);
        assert_eq!(cfg.egress_bind, "wg0");
        std::env::remove_var("PISTREAMING_HTTP_PORT");
        std::env::remove_var("PISTREAMING_EGRESS_BIND");
    }

    #[test]
    fn toml_overrides_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "http_port = 7777\ncache_max_gb = 10\n").unwrap();
        let cfg = Config::load(Some(&path)).unwrap();
        assert_eq!(cfg.http_port, 7777);
        assert_eq!(cfg.cache_max_gb, 10);
    }
}
```

- [ ] **Step 2: Correr los tests para verlos fallar**

Run: `cargo test -p pistreaming-server -- --nocapture`
Expected: FAIL — `Config` no existe.

- [ ] **Step 3: Implementar config + boot**

Reemplazar `crates/server/src/main.rs` por (arriba de los tests):
```rust
use anyhow::Context;
use pistreaming_addons::{AddonClient, AddonManager};
use pistreaming_api::{router, AppState};
use pistreaming_store::Store;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "d_data_dir")]
    pub data_dir: PathBuf,
    #[serde(default = "d_http_port")]
    pub http_port: u16,
    #[serde(default = "d_egress")]
    pub egress_bind: String,
    #[serde(default = "d_cache_max")]
    pub cache_max_gb: u64,
    #[serde(default = "d_cache_ttl")]
    pub cache_ttl_hours: u64,
}

fn d_data_dir() -> PathBuf { PathBuf::from("/data") }
fn d_http_port() -> u16 { 8000 }
fn d_egress() -> String { "eth0".into() }
fn d_cache_max() -> u64 { 40 }
fn d_cache_ttl() -> u64 { 48 }

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: d_data_dir(),
            http_port: d_http_port(),
            egress_bind: d_egress(),
            cache_max_gb: d_cache_max(),
            cache_ttl_hours: d_cache_ttl(),
        }
    }
}

impl Config {
    /// Precedencia: defaults < archivo TOML < variables de entorno `PISTREAMING_*`.
    pub fn load(path: Option<&Path>) -> anyhow::Result<Self> {
        let mut cfg = Config::default();
        if let Some(p) = path {
            if p.exists() {
                let text = std::fs::read_to_string(p)
                    .with_context(|| format!("leyendo {}", p.display()))?;
                cfg = toml::from_str(&text).context("parseando config.toml")?;
            }
        }
        if let Ok(v) = std::env::var("PISTREAMING_HTTP_PORT") {
            cfg.http_port = v.parse().context("PISTREAMING_HTTP_PORT inválido")?;
        }
        if let Ok(v) = std::env::var("PISTREAMING_EGRESS_BIND") {
            cfg.egress_bind = v;
        }
        if let Ok(v) = std::env::var("PISTREAMING_DATA_DIR") {
            cfg.data_dir = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("PISTREAMING_CACHE_MAX_GB") {
            cfg.cache_max_gb = v.parse().context("PISTREAMING_CACHE_MAX_GB inválido")?;
        }
        Ok(cfg)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg_path = std::env::var("PISTREAMING_CONFIG").ok().map(PathBuf::from);
    let cfg = Config::load(cfg_path.as_deref())?;
    tracing::info!(?cfg, "config cargada");

    std::fs::create_dir_all(&cfg.data_dir).ok();
    let store = Store::open(&cfg.data_dir.join("pistreaming.db"))?;

    let mut mgr = AddonManager::new(AddonClient::new(reqwest::Client::new()));
    for row in store.list_addons()? {
        mgr.load(&row.url, row.manifest, row.enabled);
    }
    tracing::info!(addons = mgr.addons().len(), "addons cargados");

    let state = Arc::new(AppState { store, addons: mgr });
    let app = router(state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], cfg.http_port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("escuchando en http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
```

- [ ] **Step 4: Correr los tests para verlos pasar**

Run: `cargo test -p pistreaming-server -- --nocapture`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/server
git commit -m "feat(server): config (TOML+env) y boot de la API"
```

---

## Task 11: Verificación end-to-end de la fase

**Files:** (sin cambios de código)

- [ ] **Step 1: Correr toda la suite**

Run: `cargo test --workspace`
Expected: PASS (todos los tests de core/store/addons/api/server).

- [ ] **Step 2: Lint**

Run: `cargo clippy --workspace -- -D warnings`
Expected: sin warnings. (Si hay warnings de `unused` en crates vacíos torrent/media, silenciar con `#![allow(dead_code)]` en su `lib.rs`.)

- [ ] **Step 3: Smoke manual con un addon mock**

En una terminal, levantar un manifest mock con Python:
```bash
python3 - <<'PY'
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/manifest.json":
            b = json.dumps({"id":"org.mock","version":"1.0.0","name":"Mock",
                "resources":["catalog","stream"],"types":["movie"],
                "catalogs":[{"type":"movie","id":"top","extra":[{"name":"search"}]}]}).encode()
        elif self.path == "/catalog/movie/top/search=dune.json":
            b = json.dumps({"metas":[{"id":"tt1","type":"movie","name":"Dune"}]}).encode()
        else:
            self.send_response(404); self.end_headers(); return
        self.send_response(200); self.send_header("Content-Type","application/json")
        self.send_header("Content-Length",str(len(b))); self.end_headers(); self.wfile.write(b)
HTTPServer(("127.0.0.1", 8899), H).serve_forever()
PY
```

En otra terminal, arrancar el server con `DATA_DIR` temporal:
```bash
PISTREAMING_DATA_DIR=/tmp/pistreaming-smoke PISTREAMING_HTTP_PORT=8898 cargo run -p pistreaming-server
```

Registrar el addon y buscar:
```bash
curl -s -X POST localhost:8898/api/addons -H 'content-type: application/json' \
  -d '{"url":"http://127.0.0.1:8899"}'
curl -s 'localhost:8898/api/search?query=dune'
```
Expected: el POST devuelve `{"url":...,"name":"Mock"}` con 201; el search devuelve `{"metas":[{"id":"tt1",...}]}`.

- [ ] **Step 4: Commit final de la fase (si hubo cambios de lint/stub)**

```bash
git add -A
git commit -m "chore(fase1): verificación end-to-end y limpieza de lint"
```

---

## Self-review (cobertura del spec)

| Requisito del spec | Task que lo cubre |
|---|---|
| §4 workspace de crates | Task 1 |
| §4 `core`: Manifest/Catalog/Meta/Stream/Playback | Tasks 2, 3, 4 |
| §4 `store` SQLite (addons/settings/progress/metadata) | Tasks 5, 6 |
| §4 `addons` cliente + aislamiento de addons caídos | Tasks 7, 8 |
| §4 `api` axum REST | Task 9 |
| §4 `server` config + wiring | Task 10 |
| §8 API surface (subconjunto sin play/library/UI) | Task 9 |
| §5 precedencia de config | Task 10 |
| §11 testing (unit + integración con mock) | todas |
| §6/§7/§12 (torrent, media, deploy) | **Fase 2/3/4** — fuera de este plan |

Fases siguientes (planes separados):
- **Fase 2** — `torrent` (librqbit) + `media` (ffprobe/remux) + `POST /api/play` + `GET /stream/:session` + keep/biblioteca.
- **Fase 3** — Frontend SPA vanilla embebida (addons/buscar/detalle/reproductor/biblioteca) + `/icon.svg`.
- **Fase 4** — Dockerfile multi-stage + `docker-compose.yml` con `x-casaos` + README + build en el Pi + install por `casaos-cli`.
