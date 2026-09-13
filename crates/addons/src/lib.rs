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
