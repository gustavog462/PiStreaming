//! API HTTP (axum) de PiStreaming — Fase 1.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use pistreaming_addons::AddonManager;
use pistreaming_core::error::CoreResult;
use pistreaming_store::Store;
use serde::Deserialize;
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
