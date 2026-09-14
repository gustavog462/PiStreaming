//! Cliente HTTP del protocolo de addons de Stremio.

use pistreaming_core::catalog::CatalogRequest;
use pistreaming_core::error::{CoreError, CoreResult};
use pistreaming_core::manifest::Manifest;
use pistreaming_core::meta::{MetaDetail, MetaItem};
use pistreaming_core::normalize_url;
use pistreaming_core::stream::Stream;
use pistreaming_store::Store;
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
        normalize_url(url)
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

    /// Construye un manager cargando los addons ya persistidos en `store`,
    /// reusando el `AddonClient` configurado (no fabrica uno nuevo).
    pub async fn load(client: AddonClient, store: &Store) -> CoreResult<Self> {
        let mut mgr = Self::new(client);
        for row in store.list_addons()? {
            mgr.push(&row.url, row.manifest, row.enabled);
        }
        Ok(mgr)
    }

    pub fn addons(&self) -> &[AddonRef] {
        &self.addons
    }

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

    /// Registra un addon fetcheando su manifest. Falla si el manifest no carga.
    pub async fn add_from_url(&mut self, url: &str) -> CoreResult<()> {
        let manifest = self.client.fetch_manifest(url).await?;
        self.addons.push(AddonRef {
            url: normalize_url(url),
            manifest,
            enabled: true,
        });
        Ok(())
    }

    /// Carga addons desde filas de store (manifest ya cacheado).
    pub fn push(&mut self, url: &str, manifest: Manifest, enabled: bool) {
        self.addons.push(AddonRef {
            url: normalize_url(url),
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
    async fn fetch_manifest_acepta_url_de_manifest() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/manifest.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "org.test", "version": "1.0.0", "name": "Test",
                "resources": ["catalog", "meta", "stream"],
                "types": ["movie"],
                "catalogs": []
            })))
            .mount(&server)
            .await;

        let client = AddonClient::new(reqwest::Client::new());
        // El usuario pega la URL del manifest: normalize_url debe quitar el
        // sufijo para no pedir `/manifest.json/manifest.json`.
        let m = client
            .fetch_manifest(&format!("{}/manifest.json", server.uri()))
            .await
            .unwrap();
        assert_eq!(m.id, "org.test");
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
}
