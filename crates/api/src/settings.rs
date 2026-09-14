//! Ajustes de la app: los que aplican en caliente (atomics) y los que exigen reinicio.

use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::Value;

use crate::SharedState;

/// Claves persistidas en la tabla `settings`.
pub const KEY_CACHE_MAX_GB: &str = "cache_max_gb";
pub const KEY_CACHE_TTL_HOURS: &str = "cache_ttl_hours";
pub const KEY_EGRESS_BIND: &str = "egress_bind";
pub const KEY_HTTP_PORT: &str = "http_port";
pub const KEY_DATA_DIR: &str = "data_dir";

/// Ajustes que el evictor lee en cada tick (aplican sin reiniciar).
pub struct RuntimeSettings {
    cache_max_gb: AtomicU64,
    cache_ttl_hours: AtomicU64,
}

impl RuntimeSettings {
    pub fn new(cache_max_gb: u64, cache_ttl_hours: u64) -> Self {
        Self {
            cache_max_gb: AtomicU64::new(cache_max_gb),
            cache_ttl_hours: AtomicU64::new(cache_ttl_hours),
        }
    }

    pub fn cache_max_gb(&self) -> u64 {
        self.cache_max_gb.load(Ordering::Relaxed)
    }

    pub fn cache_ttl_hours(&self) -> u64 {
        self.cache_ttl_hours.load(Ordering::Relaxed)
    }

    pub fn set_cache_max_gb(&self, v: u64) {
        self.cache_max_gb.store(v, Ordering::Relaxed);
    }

    pub fn set_cache_ttl_hours(&self, v: u64) {
        self.cache_ttl_hours.store(v, Ordering::Relaxed);
    }
}

/// Valores activos de arranque para los campos que solo aplican reiniciando.
#[derive(Clone)]
pub struct StaticSettings {
    pub egress_bind: String,
    pub http_port: u16,
    pub data_dir: std::path::PathBuf,
}

/// GET /api/settings — valores efectivos + qué campo exige reinicio.
pub async fn get_settings(State(st): State<SharedState>) -> Response {
    let persisted = |key: &str| st.store.get_setting(key).ok().flatten();
    let egress_bind = persisted(KEY_EGRESS_BIND)
        .unwrap_or_else(|| st.static_settings.egress_bind.clone());
    let data_dir = persisted(KEY_DATA_DIR)
        .unwrap_or_else(|| st.static_settings.data_dir.to_string_lossy().into_owned());
    let http_port = persisted(KEY_HTTP_PORT)
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(st.static_settings.http_port);
    Json(serde_json::json!({
        "cache_max_gb": st.settings.cache_max_gb(),
        "cache_ttl_hours": st.settings.cache_ttl_hours(),
        "egress_bind": egress_bind,
        "data_dir": data_dir,
        "http_port": http_port,
        "requires_restart": [KEY_EGRESS_BIND, KEY_HTTP_PORT, KEY_DATA_DIR],
    }))
    .into_response()
}

fn field_err(field: &str) -> Response {
    crate::err(
        StatusCode::BAD_REQUEST,
        format!("valor inválido en `{field}`"),
    )
}

/// PUT /api/settings — aplica `cache_max_gb`/`cache_ttl_hours` en caliente;
/// persiste `egress_bind`/`http_port`/`data_dir` y los marca como reinicio.
pub async fn put_settings(
    State(st): State<SharedState>,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    let Json(v) = match body {
        Ok(b) => b,
        Err(_) => return crate::err(StatusCode::BAD_REQUEST, "body inválido"),
    };
    let Some(obj) = v.as_object() else {
        return crate::err(StatusCode::BAD_REQUEST, "body inválido");
    };

    let mut applied: Vec<&str> = Vec::new();
    let mut requires_restart: Vec<&str> = Vec::new();

    if let Some(val) = obj.get(KEY_CACHE_MAX_GB) {
        let Some(n) = val.as_u64().filter(|n| *n > 0) else {
            return field_err(KEY_CACHE_MAX_GB);
        };
        st.settings.set_cache_max_gb(n);
        if let Err(e) = st.store.set_setting(KEY_CACHE_MAX_GB, &n.to_string()) {
            return crate::core_err(e);
        }
        applied.push(KEY_CACHE_MAX_GB);
    }
    if let Some(val) = obj.get(KEY_CACHE_TTL_HOURS) {
        let Some(n) = val.as_u64().filter(|n| *n > 0) else {
            return field_err(KEY_CACHE_TTL_HOURS);
        };
        st.settings.set_cache_ttl_hours(n);
        if let Err(e) = st.store.set_setting(KEY_CACHE_TTL_HOURS, &n.to_string()) {
            return crate::core_err(e);
        }
        applied.push(KEY_CACHE_TTL_HOURS);
    }
    if let Some(val) = obj.get(KEY_EGRESS_BIND) {
        let Some(s) = val.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
            return field_err(KEY_EGRESS_BIND);
        };
        if let Err(e) = st.store.set_setting(KEY_EGRESS_BIND, s) {
            return crate::core_err(e);
        }
        requires_restart.push(KEY_EGRESS_BIND);
    }
    if let Some(val) = obj.get(KEY_HTTP_PORT) {
        let Some(n) = val.as_u64().filter(|n| (1..=65535).contains(n)) else {
            return field_err(KEY_HTTP_PORT);
        };
        if let Err(e) = st.store.set_setting(KEY_HTTP_PORT, &n.to_string()) {
            return crate::core_err(e);
        }
        requires_restart.push(KEY_HTTP_PORT);
    }
    if let Some(val) = obj.get(KEY_DATA_DIR) {
        let Some(s) = val.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
            return field_err(KEY_DATA_DIR);
        };
        if let Err(e) = st.store.set_setting(KEY_DATA_DIR, s) {
            return crate::core_err(e);
        }
        requires_restart.push(KEY_DATA_DIR);
    }

    Json(serde_json::json!({ "applied": applied, "requires_restart": requires_restart }))
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn app_con(tmp: &tempfile::TempDir) -> (SharedState, axum::Router) {
        let st = crate::test_state(tmp.path().to_path_buf());
        let app = crate::router(st.clone());
        (st, app)
    }

    #[tokio::test]
    async fn put_aplica_caliente_y_marca_reinicio() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, app) = app_con(&tmp);

        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/settings")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"cache_max_gb":12,"http_port":9090}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["applied"], serde_json::json!(["cache_max_gb"]));
        assert_eq!(v["requires_restart"], serde_json::json!(["http_port"]));

        // En caliente: el runtime ya lo refleja sin reiniciar.
        assert_eq!(st.settings.cache_max_gb(), 12);
        assert_eq!(st.settings.cache_ttl_hours(), 48);

        // GET refleja lo persistido y lista los campos de reinicio.
        let res = app
            .oneshot(Request::get("/api/settings").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["cache_max_gb"], 12);
        assert_eq!(v["http_port"], 9090);
        assert_eq!(
            v["requires_restart"],
            serde_json::json!(["egress_bind", "http_port", "data_dir"])
        );
    }

    #[tokio::test]
    async fn put_valor_invalido_es_400_con_el_campo() {
        let tmp = tempfile::tempdir().unwrap();
        let (_st, app) = app_con(&tmp);

        let res = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/settings")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"cache_max_gb":"mucho"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            v["error"].as_str().unwrap().contains("cache_max_gb"),
            "el 400 debe indicar el campo: {v}"
        );
    }

    #[test]
    fn runtime_settings_set_y_get() {
        let s = RuntimeSettings::new(10, 24);
        assert_eq!(s.cache_max_gb(), 10);
        assert_eq!(s.cache_ttl_hours(), 24);
        s.set_cache_max_gb(99);
        s.set_cache_ttl_hours(3);
        assert_eq!(s.cache_max_gb(), 99);
        assert_eq!(s.cache_ttl_hours(), 3);
    }
}
