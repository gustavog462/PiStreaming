//! API HTTP (axum) de PiStreaming — Fase 1.

use axum::extract::rejection::JsonRejection;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use pistreaming_addons::{AddonClient, AddonManager};
use pistreaming_core::error::{CoreError, CoreResult};
use pistreaming_core::normalize_url;
use pistreaming_store::Store;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

pub mod range;

pub struct AppState {
    pub store: Store,
    /// Cliente HTTP configurado, creado una sola vez y reusado al recargar el manager.
    pub client: AddonClient,
    pub addons: Arc<RwLock<AddonManager>>,
    /// Serializa las mutaciones (store write + reload + swap) de alta/baja.
    pub mutex: Mutex<()>,
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

/// Mapea un `CoreError` a status HTTP, loguea el detalle y responde un mensaje
/// genérico (no filtra SQLite/HTTP al cliente). Forma `{ error, code }` (spec §10).
fn core_err(e: CoreError) -> Response {
    let (code, msg) = match &e {
        CoreError::NotFound(_) => (StatusCode::NOT_FOUND, "no encontrado"),
        CoreError::Http(_) | CoreError::Json(_) => {
            (StatusCode::BAD_GATEWAY, "error del addon upstream")
        }
        CoreError::Unsupported(_) => (StatusCode::UNPROCESSABLE_ENTITY, "no soportado"),
        CoreError::Db(_) | CoreError::Addon(_) | CoreError::Other(_) => {
            (StatusCode::INTERNAL_SERVER_ERROR, "error interno")
        }
    };
    tracing::error!(error = %e, status = code.as_u16(), "request falló");
    err(code, msg)
}

pub async fn health() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

pub async fn list_addons(State(st): State<SharedState>) -> Response {
    match st.store.list_addons() {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => core_err(e),
    }
}

#[derive(Deserialize)]
pub struct AddAddonBody {
    pub url: String,
}

pub async fn add_addon(
    State(st): State<SharedState>,
    body: Result<Json<AddAddonBody>, JsonRejection>,
) -> Response {
    let Json(body) = match body {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "body inválido");
            return err(StatusCode::BAD_REQUEST, "body inválido");
        }
    };
    let url = normalize_url(&body.url);
    match fetch_and_store(&st, &url).await {
        Ok(name) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "url": url, "name": name })),
        )
            .into_response(),
        Err(e) => core_err(e),
    }
}

async fn fetch_and_store(st: &SharedState, url: &str) -> CoreResult<String> {
    // Fetch del manifest fuera del lock de estado (M5).
    let manifest = st.client.fetch_manifest(url).await?;
    // Serializa la mutación store→reload→swap (M6).
    let _guard = st.mutex.lock().await;
    st.store.add_addon(url, &manifest)?;
    let fresh = AddonManager::load(st.client.clone(), &st.store).await?;
    *st.addons.write().await = fresh;
    Ok(manifest.name)
}

pub async fn remove_addon(
    State(st): State<SharedState>,
    axum::extract::Path(url): axum::extract::Path<String>,
) -> Response {
    let url = normalize_url(&decode_url(&url));
    let _guard = st.mutex.lock().await;
    match st.store.remove_addon(&url) {
        Ok(true) => match AddonManager::load(st.client.clone(), &st.store).await {
            Ok(fresh) => {
                *st.addons.write().await = fresh;
                StatusCode::NO_CONTENT.into_response()
            }
            Err(e) => core_err(e),
        },
        Ok(false) => err(StatusCode::NOT_FOUND, "no encontrado"),
        Err(e) => core_err(e),
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
    // Snapshot del manager: suelta el read guard antes del I/O de red (M5).
    let mgr = st.addons.read().await.clone();
    let metas = mgr.search(&q.kind, &query).await;
    Json(serde_json::json!({ "metas": metas })).into_response()
}

pub async fn meta(
    State(st): State<SharedState>,
    axum::extract::Path((kind, id)): axum::extract::Path<(String, String)>,
) -> Response {
    let mgr = st.addons.read().await.clone();
    match mgr.meta(&kind, &id).await {
        Ok(m) => Json(m).into_response(),
        Err(e) => core_err(e),
    }
}

pub async fn streams(
    State(st): State<SharedState>,
    axum::extract::Path((kind, id)): axum::extract::Path<(String, String)>,
) -> Response {
    let mgr = st.addons.read().await.clone();
    let streams = mgr.streams(&kind, &id).await;
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
        Mock::given(method("GET"))
            .and(path("/stream/movie/tt1.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "streams": [{"name": "Torrentio", "title": "1080p 👤 900", "infoHash": "abc123"}]
            })))
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();
        let client = AddonClient::new(reqwest::Client::new());
        let mut mgr = AddonManager::new(client.clone());
        mgr.add_from_url(&server.uri()).await.unwrap();

        Arc::new(AppState {
            store,
            client,
            addons: Arc::new(RwLock::new(mgr)),
            mutex: Mutex::new(()),
        })
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

    /// Regresión: un addon registrado por POST debe quedar visible al search
    /// en la misma sesión (sin reiniciar el server).
    #[tokio::test]
    async fn add_addon_is_reflected_in_search_without_restart() {
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

        // store y manager arrancan vacíos: el alta debe poblar el manager en memoria.
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();
        let client = AddonClient::new(reqwest::Client::new());
        let mgr = AddonManager::new(client.clone());
        let app = router(Arc::new(AppState {
            store,
            client,
            addons: Arc::new(RwLock::new(mgr)),
            mutex: Mutex::new(()),
        }));

        let body = serde_json::json!({ "url": server.uri() }).to_string();
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/addons")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let resp = app
            .oneshot(Request::get("/api/search?query=dune").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            v["metas"][0]["id"], "tt1",
            "el addon registrado debe reflejarse en el search sin reiniciar: {v}"
        );
    }

    /// Codifica una URL para un único segmento de path (`:url`).
    fn enc(u: &str) -> String {
        u.replace(':', "%3A").replace('/', "%2F")
    }

    #[tokio::test]
    async fn streams_json_includes_source_addon() {
        let app = router(test_state().await);
        let resp = app
            .oneshot(Request::get("/api/streams/movie/tt1").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["streams"][0]["infoHash"], "abc123", "el stream debe aparecer: {v}");
        assert_eq!(v["streams"][0]["source_addon"], "Mock", "source_addon debe serializarse: {v}");
    }

    #[tokio::test]
    async fn post_with_trailing_slash_then_delete_without_slash() {
        let m = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/manifest.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "org.mock2", "version": "1.0.0", "name": "Mock2",
                "resources": ["catalog"], "types": ["movie"],
                "catalogs": [{"type": "movie", "id": "top", "extra": [{"name": "search"}]}]
            })))
            .mount(&m)
            .await;

        let app = router(test_state().await);

        // POST con barra final
        let body = serde_json::json!({ "url": format!("{}/", m.uri()) }).to_string();
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/addons")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        // listado: una sola fila y URL normalizada (sin barra)
        let resp = app
            .clone()
            .oneshot(Request::get("/api/addons").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(rows.len(), 1, "no debe duplicar por barra: {rows:?}");
        assert_eq!(rows[0]["url"], m.uri(), "URL guardada normalizada: {rows:?}");

        // DELETE sin barra
        let resp = app
            .clone()
            .oneshot(
                Request::delete(format!("/api/addons/{}", enc(&m.uri())))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // ya no se lista
        let resp = app
            .clone()
            .oneshot(Request::get("/api/addons").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let rows: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
        assert!(rows.is_empty(), "el addon borrado no debe listarse: {rows:?}");

        // y el search federado queda vacío
        let resp = app
            .oneshot(Request::get("/api/search?query=dune").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["metas"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn delete_missing_addon_is_404() {
        let app = router(test_state().await);
        let resp = app
            .oneshot(
                Request::delete(format!("/api/addons/{}", enc("http://127.0.0.1:1")))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn add_addon_upstream_error_is_502() {
        let upstream = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/manifest.json"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&upstream)
            .await;

        let app = router(test_state().await);
        let body = serde_json::json!({ "url": upstream.uri() }).to_string();
        let resp = app
            .oneshot(
                Request::post("/api/addons")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn add_addon_invalid_body_is_400() {
        let app = router(test_state().await);
        let resp = app
            .oneshot(
                Request::post("/api/addons")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn error_mapping_statuses() {
        let cases = [
            (CoreError::NotFound("x".into()), StatusCode::NOT_FOUND),
            (CoreError::Http("x".into()), StatusCode::BAD_GATEWAY),
            (CoreError::Json("x".into()), StatusCode::BAD_GATEWAY),
            (CoreError::Db("x".into()), StatusCode::INTERNAL_SERVER_ERROR),
            (CoreError::Addon("x".into()), StatusCode::INTERNAL_SERVER_ERROR),
            (CoreError::Other("x".into()), StatusCode::INTERNAL_SERVER_ERROR),
        ];
        for (e, expected) in cases {
            assert_eq!(core_err(e).status(), expected);
        }
    }
}
