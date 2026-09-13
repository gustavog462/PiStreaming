use anyhow::Context;
use pistreaming_addons::{AddonClient, AddonManager};
use pistreaming_api::{router, AppState};
use pistreaming_store::Store;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

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
        let toml_text = match path {
            Some(p) if p.exists() => Some(
                std::fs::read_to_string(p).with_context(|| format!("leyendo {}", p.display()))?,
            ),
            _ => None,
        };
        let env: BTreeMap<String, String> = std::env::vars().collect();
        Self::from_sources(toml_text.as_deref(), &env)
    }

    /// Función pura (sin I/O ni env global): testeable y sin races.
    // TODO(fase4): reconciliar nombres con spec §12 (hoy `PISTREAMING_*` en vez de
    // `EGRESS_BIND` / `CACHE_TTL_HOURS` / `DATA_DIR`).
    pub fn from_sources(
        toml_text: Option<&str>,
        env: &BTreeMap<String, String>,
    ) -> anyhow::Result<Self> {
        let mut cfg = Config::default();
        if let Some(text) = toml_text {
            cfg = toml::from_str(text).context("parseando config.toml")?;
        }
        if let Some(v) = env.get("PISTREAMING_HTTP_PORT") {
            cfg.http_port = v.parse().context("PISTREAMING_HTTP_PORT inválido")?;
        }
        if let Some(v) = env.get("PISTREAMING_EGRESS_BIND") {
            cfg.egress_bind = v.clone();
        }
        if let Some(v) = env.get("PISTREAMING_DATA_DIR") {
            cfg.data_dir = PathBuf::from(v);
        }
        if let Some(v) = env.get("PISTREAMING_CACHE_MAX_GB") {
            cfg.cache_max_gb = v.parse().context("PISTREAMING_CACHE_MAX_GB inválido")?;
        }
        if let Some(v) = env.get("PISTREAMING_CACHE_TTL_HOURS") {
            cfg.cache_ttl_hours = v.parse().context("PISTREAMING_CACHE_TTL_HOURS inválido")?;
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

    let client = AddonClient::new(reqwest::Client::new());
    let mgr = AddonManager::load(client.clone(), &store).await?;
    tracing::info!(addons = mgr.addons().len(), "addons cargados");

    use pistreaming_torrent::open_session;

    let cache_dir = cfg.data_dir.join("cache");
    std::fs::create_dir_all(&cache_dir).ok();

    let torrent_session = open_session(cache_dir.clone(), Some(cfg.egress_bind.clone())).await?;
    tracing::info!(bind = %cfg.egress_bind, "sesión torrent abierta");

    let registry = pistreaming_api::session::PlaySessionRegistry::new();
    let torrents_cell = tokio::sync::OnceCell::new();
    let _ = torrents_cell.set(torrent_session);

    let state = Arc::new(AppState {
        store,
        client,
        addons: Arc::new(RwLock::new(mgr)),
        mutex: Mutex::new(()),
        torrents: torrents_cell,
        cache_dir: cache_dir.clone(),
        public_base: format!("http://127.0.0.1:{}", cfg.http_port),
        sessions: registry,
        handles: parking_lot::RwLock::new(Default::default()),
    });

    // Job de eviction: cada 10 min, borra sesiones inactivas fuera de TTL o que
    // excedan CACHE_MAX_GB. Nunca toca las sesiones activas del registro.
    let evict_state = state.clone();
    let cache_max_gb = cfg.cache_max_gb;
    let cache_ttl_hours = cfg.cache_ttl_hours;
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(600));
        loop {
            tick.tick().await;
            if let Err(e) = evict_cache(&evict_state, cache_max_gb, cache_ttl_hours).await {
                tracing::warn!(error = %e, "eviction falló");
            }
        }
    });

    let app = router(state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], cfg.http_port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("escuchando en http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Eviction LRU por tamaño/TTL. No borra carpetas de sesiones activas.
async fn evict_cache(
    state: &pistreaming_api::SharedState,
    cache_max_gb: u64,
    cache_ttl_hours: u64,
) -> anyhow::Result<()> {
    use std::time::{Duration, SystemTime};

    let active: std::collections::HashSet<String> = state.sessions.active_ids().into_iter().collect();
    let root = &state.cache_dir;
    let mut dirs: Vec<(std::path::PathBuf, u64, SystemTime)> = Vec::new();
    let mut total: u64 = 0;

    let mut entries = tokio::fs::read_dir(root).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if !entry.file_type().await?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if active.iter().any(|a| a.starts_with(&name) || a.contains(&name)) {
            continue; // sesión activa
        }
        let mut size = 0u64;
        let mut newest = SystemTime::UNIX_EPOCH;
        let mut stack = vec![path.clone()];
        while let Some(p) = stack.pop() {
            let mut rd = match tokio::fs::read_dir(&p).await { Ok(r) => r, Err(_) => continue };
            while let Some(f) = rd.next_entry().await.ok().flatten() {
                let md = match f.metadata().await { Ok(m) => m, Err(_) => continue };
                if md.is_dir() {
                    stack.push(f.path());
                } else {
                    size += md.len();
                    if let Ok(t) = md.modified() {
                        if t > newest { newest = t; }
                    }
                }
            }
        }
        total += size;
        dirs.push((path, size, newest));
    }

    // Orden LRU: los más viejos primero.
    dirs.sort_by_key(|(_, _, t)| *t);

    let ttl = Duration::from_secs(cache_ttl_hours * 3600);
    let now = SystemTime::now();
    for (path, size, modified) in &dirs {
        let too_old = now.duration_since(*modified).map(|d| d > ttl).unwrap_or(false);
        let too_big = total > cache_max_gb * 1024 * 1024 * 1024;
        if too_old || too_big {
            tracing::info!(dir = %path.display(), "evictando caché");
            tokio::fs::remove_dir_all(path).await.ok();
            total = total.saturating_sub(*size);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Config;
    use std::collections::BTreeMap;

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn env_overrides_defaults() {
        let e = env(&[("PISTREAMING_HTTP_PORT", "9999"), ("PISTREAMING_EGRESS_BIND", "wg0")]);
        let cfg = Config::from_sources(None, &e).unwrap();
        assert_eq!(cfg.http_port, 9999);
        assert_eq!(cfg.egress_bind, "wg0");
    }

    #[test]
    fn toml_overrides_defaults() {
        let cfg = Config::from_sources(
            Some("http_port = 7777\ncache_max_gb = 10\n"),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(cfg.http_port, 7777);
        assert_eq!(cfg.cache_max_gb, 10);
    }

    #[test]
    fn env_overrides_toml_and_reads_ttl() {
        let e = env(&[("PISTREAMING_CACHE_TTL_HOURS", "5")]);
        let cfg = Config::from_sources(Some("cache_ttl_hours = 99\n"), &e).unwrap();
        assert_eq!(cfg.cache_ttl_hours, 5);
    }

    #[test]
    fn default_ttl_matches_config_default() {
        let cfg = Config::from_sources(None, &BTreeMap::new()).unwrap();
        assert_eq!(cfg.cache_ttl_hours, 48);
    }
}
