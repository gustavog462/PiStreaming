//! API HTTP (axum) de PiStreaming — Fase 1.

use crate::session::{PlaySession, PlaySessionRegistry};
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use pistreaming_addons::{AddonClient, AddonManager};
use pistreaming_core::error::{CoreError, CoreResult};
use pistreaming_core::normalize_url;
use pistreaming_core::playback::PlaybackRoute;
use pistreaming_media::{decide, probe};
use pistreaming_store::Store;
use pistreaming_torrent::{add_magnet, pick_largest_video};
use serde::Deserialize;
use std::sync::Arc;
use std::sync::Arc as StdArc;
use tokio::sync::{Mutex, RwLock};

pub mod range;
pub mod session;

pub struct AppState {
    pub store: Store,
    /// Cliente HTTP configurado, creado una sola vez y reusado al recargar el manager.
    pub client: AddonClient,
    pub addons: Arc<RwLock<AddonManager>>,
    /// Serializa las mutaciones (store write + reload + swap) de alta/baja.
    pub mutex: Mutex<()>,
    /// Sesión de librqbit (None hasta que el server la crea tras el arranque).
    pub torrents: tokio::sync::OnceCell<StdArc<librqbit::Session>>,
    /// Raíz de caché de torrents; cada magnet escribe bajo `cache_dir/<info_hash>`.
    pub cache_dir: std::path::PathBuf,
    /// Base pública para construir las URLs `/raw` y `/stream`.
    pub public_base: String,
    /// Registro en memoria de sesiones de reproducción activas.
    pub sessions: PlaySessionRegistry,
    /// Handles de torrent vivos por session id, para que `/raw` y `/stream` los resuelvan.
    ///
    /// Se usa el tipo concreto `Arc<librqbit::ManagedTorrent>` (Opción A) porque
    /// `librqbit::ManagedTorrentHandle` no está re-exportado en la raíz de 9.0.1
    /// (vive en el módulo privado `torrent_state`). `ManagedTorrent` sí es público.
    pub handles:
        parking_lot::RwLock<std::collections::HashMap<String, StdArc<librqbit::ManagedTorrent>>>,
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
        .route("/api/play", axum::routing::post(play))
        .route("/raw/:session", get(raw_stream))
        .route("/stream/:session", get(stream))
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

#[derive(Deserialize)]
pub struct PlayBody {
    pub magnet: String,
    #[serde(default)]
    pub title: Option<String>,
}

/// POST /api/play { magnet } -> PlaybackPlan
pub async fn play(
    State(st): State<SharedState>,
    body: Result<Json<PlayBody>, JsonRejection>,
) -> Response {
    let Json(body) = match body {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = %e, "body inválido");
            return err(StatusCode::BAD_REQUEST, "body inválido");
        }
    };
    let session = match st.torrents.get() {
        Some(s) => s.clone(),
        None => {
            return err(
                StatusCode::SERVICE_UNAVAILABLE,
                "engine de torrents no inicializado",
            )
        }
    };

    // 1) agregar el magnet (crea la carpeta de caché por info_hash)
    let added = match add_magnet(&session, &body.magnet, &st.cache_dir).await {
        Ok(a) => a,
        Err(e) => return core_err(e),
    };
    let cache_dir = st.cache_dir.join(&added.info_hash);
    std::fs::create_dir_all(&cache_dir).ok();

    // 2) elegir archivo de video
    let (file_id, _name, _len) = match pick_largest_video(&added.handle) {
        Ok(v) => v,
        Err(e) => return core_err(e),
    };

    // 3) probe (la sesión se inserta al final)
    let session_id = uuid_like(&added.info_hash, file_id);
    let raw_url = format!("{}/raw/{}", st.public_base, session_id);
    let playback_url = format!("{}/stream/{}", st.public_base, session_id);

    let probe_input = local_probe_path(&added.handle, file_id);
    let probe_result = match probe_input {
        Some(path) => probe(path.to_str().unwrap()).await,
        None => probe(&raw_url).await,
    };
    let p = match probe_result {
        Ok(p) => p,
        Err(e) => return core_err(e),
    };

    let mut plan = match decide(&session_id, &p, &playback_url, Some(&raw_url)) {
        Ok(plan) => plan,
        Err(e) => return core_err(e),
    };
    if plan.route == PlaybackRoute::Direct {
        plan.playback_url = raw_url.clone();
    }

    st.sessions.insert(PlaySession {
        id: session_id.clone(),
        info_hash: added.info_hash.clone(),
        file_id,
        plan: plan.clone(),
        cache_dir,
        created_at: std::time::Instant::now(),
        ffmpeg: None,
    });
    st.handles.write().insert(session_id.clone(), added.handle);

    (StatusCode::OK, Json(plan)).into_response()
}

/// GET /raw/:session — bytes crudos del FileStream con Range (fuente para ffmpeg).
pub async fn raw_stream(
    State(st): State<SharedState>,
    Path(session): Path<String>,
    headers: HeaderMap,
) -> Response {
    let handle = match st.handles.read().get(&session).cloned() {
        Some(h) => h,
        None => return err(StatusCode::NOT_FOUND, "sesión desconocida"),
    };
    let file_id = match st.sessions.get(&session) {
        Some(s) => s.read().file_id,
        None => return err(StatusCode::NOT_FOUND, "sesión desconocida"),
    };
    let len = match file_len(&handle, file_id) {
        Ok(l) => l,
        Err(e) => return core_err(e),
    };
    let stream = match pistreaming_torrent::torrent_stream(&handle, file_id).await {
        Ok(s) => s,
        Err(e) => return core_err(e),
    };
    crate::range::ranged_response(stream, len, &headers, "application/octet-stream").await
}

/// GET /stream/:session — lo que consume el <video>. Directo: sirve el FileStream.
/// (Remux/RecodeAudio se completan en la Task 10.)
pub async fn stream(
    State(st): State<SharedState>,
    Path(session): Path<String>,
    headers: HeaderMap,
) -> Response {
    let handle = match st.handles.read().get(&session).cloned() {
        Some(h) => h,
        None => return err(StatusCode::NOT_FOUND, "sesión desconocida"),
    };
    let (file_id, route) = match st.sessions.get(&session) {
        Some(s) => {
            let g = s.read();
            (g.file_id, g.plan.route)
        }
        None => return err(StatusCode::NOT_FOUND, "sesión desconocida"),
    };

    match route {
        PlaybackRoute::Direct => {
            let len = match file_len(&handle, file_id) {
                Ok(l) => l,
                Err(e) => return core_err(e),
            };
            let stream = match pistreaming_torrent::torrent_stream(&handle, file_id).await {
                Ok(s) => s,
                Err(e) => return core_err(e),
            };
            let mime = if session_mime(&st, &session).contains("webm") {
                "video/webm"
            } else {
                "video/mp4"
            };
            crate::range::ranged_response(stream, len, &headers, mime).await
        }
        PlaybackRoute::Remux | PlaybackRoute::RecodeAudio => {
            // Task 10.
            err(StatusCode::NOT_IMPLEMENTED, "remux aún no implementado")
        }
    }
}

fn session_mime(_st: &SharedState, _session: &str) -> String {
    "video/mp4".to_string()
}

/// Largo del archivo del torrent.
///
/// `torrent_stream` devuelve un RPIT opaco (`impl AsyncRead + AsyncSeek + ...`)
/// que no expone `.len()`, y `librqbit::FileStream` no es nombrable (módulo
/// privado), así que el largo se consulta a la metadata del handle.
fn file_len(handle: &StdArc<librqbit::ManagedTorrent>, file_id: usize) -> Result<u64, CoreError> {
    match handle.with_metadata(|m| m.file_infos.get(file_id).map(|f| f.len)) {
        Ok(Some(l)) => Ok(l),
        Ok(None) => Err(CoreError::NotFound(
            "archivo no encontrado en el torrent".into(),
        )),
        Err(e) => Err(CoreError::Other(format!("metadata no disponible: {e}"))),
    }
}

/// Ruta local del archivo dentro de la carpeta de salida de librqbit, si existe.
fn local_probe_path(
    handle: &StdArc<librqbit::ManagedTorrent>,
    file_id: usize,
) -> Option<std::path::PathBuf> {
    handle
        .with_metadata(|m| {
            let f = m.file_infos.get(file_id)?;
            Some(f.relative_filename.clone())
        })
        .ok()
        .flatten()
        .map(|rel| handle.output_folder().join(rel))
}

fn uuid_like(info_hash: &str, file_id: usize) -> String {
    format!("{}-{}", &info_hash[..info_hash.len().min(12)], file_id)
}

/// Construye un AppState sin engine de torrents (test-only).
#[doc(hidden)]
pub fn test_state(dir: std::path::PathBuf) -> SharedState {
    let store = Store::open(&dir.join("pistreaming.db")).expect("store");
    let client = AddonClient::new(reqwest::Client::new());
    let addons = AddonManager::new(client.clone());
    StdArc::new(AppState {
        store,
        client,
        addons: StdArc::new(RwLock::new(addons)),
        mutex: Mutex::new(()),
        torrents: tokio::sync::OnceCell::new(),
        cache_dir: dir,
        public_base: "http://127.0.0.1:8000".to_string(),
        sessions: PlaySessionRegistry::new(),
        handles: parking_lot::RwLock::new(std::collections::HashMap::new()),
    })
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
            torrents: tokio::sync::OnceCell::new(),
            cache_dir: dir.path().to_path_buf(),
            public_base: "http://127.0.0.1:8000".to_string(),
            sessions: PlaySessionRegistry::new(),
            handles: parking_lot::RwLock::new(std::collections::HashMap::new()),
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
            .oneshot(
                Request::get("/api/search?query=dune")
                    .body(Body::empty())
                    .unwrap(),
            )
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
            torrents: tokio::sync::OnceCell::new(),
            cache_dir: dir.path().to_path_buf(),
            public_base: "http://127.0.0.1:8000".to_string(),
            sessions: PlaySessionRegistry::new(),
            handles: parking_lot::RwLock::new(std::collections::HashMap::new()),
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
            .oneshot(
                Request::get("/api/search?query=dune")
                    .body(Body::empty())
                    .unwrap(),
            )
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
            .oneshot(
                Request::get("/api/streams/movie/tt1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            v["streams"][0]["infoHash"], "abc123",
            "el stream debe aparecer: {v}"
        );
        assert_eq!(
            v["streams"][0]["source_addon"], "Mock",
            "source_addon debe serializarse: {v}"
        );
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
        assert_eq!(
            rows[0]["url"],
            m.uri(),
            "URL guardada normalizada: {rows:?}"
        );

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
        assert!(
            rows.is_empty(),
            "el addon borrado no debe listarse: {rows:?}"
        );

        // y el search federado queda vacío
        let resp = app
            .oneshot(
                Request::get("/api/search?query=dune")
                    .body(Body::empty())
                    .unwrap(),
            )
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
            (
                CoreError::Addon("x".into()),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            (
                CoreError::Other("x".into()),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];
        for (e, expected) in cases {
            assert_eq!(core_err(e).status(), expected);
        }
    }
}
