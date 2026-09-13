use anyhow::Context;
use pistreaming_addons::AddonManager;
use pistreaming_api::{router, AppState};
use pistreaming_store::Store;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

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

    let mgr = AddonManager::load(&store).await?;
    tracing::info!(addons = mgr.addons().len(), "addons cargados");

    let state = Arc::new(AppState { store, addons: Arc::new(RwLock::new(mgr)) });
    let app = router(state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], cfg.http_port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("escuchando en http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

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
