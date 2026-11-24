//! Métricas Prometheus (formato de texto, sin dependencias externas).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use axum::extract::{MatchedPath, Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use parking_lot::Mutex;

use crate::SharedState;

pub struct Metrics {
    requests: Mutex<HashMap<(String, String, u16), u64>>,
    pub cache_bytes: AtomicU64,
    started: Instant,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            requests: Mutex::new(HashMap::new()),
            cache_bytes: AtomicU64::new(0),
            started: Instant::now(),
        }
    }

    pub fn record(&self, method: &str, path: &str, status: u16) {
        *self
            .requests
            .lock()
            .entry((method.to_string(), path.to_string(), status))
            .or_insert(0) += 1;
    }

    pub fn render(&self, st: &SharedState) -> String {
        let mut out = String::new();
        out.push_str("# HELP pistreaming_requests_total Peticiones HTTP.\n");
        out.push_str("# TYPE pistreaming_requests_total counter\n");
        let reqs = self.requests.lock();
        let mut keys: Vec<_> = reqs.keys().cloned().collect();
        keys.sort();
        for (method, path, status) in keys {
            let n = reqs
                .get(&(method.clone(), path.clone(), status))
                .copied()
                .unwrap_or(0);
            out.push_str(&format!(
                "pistreaming_requests_total{{method=\"{method}\",path=\"{path}\",status=\"{status}\"}} {n}\n"
            ));
        }

        let sessions = st.sessions.len();
        let torrents = st.handles.read().len();
        let ffmpeg = st
            .sessions
            .active_ids()
            .iter()
            .filter(|id| {
                st.sessions
                    .get(id)
                    .map(|s| s.read().ffmpeg.is_some())
                    .unwrap_or(false)
            })
            .count()
            + crate::hls::active_count();
        let cache = self.cache_bytes.load(Ordering::Relaxed);
        let uptime = self.started.elapsed().as_secs();

        out.push_str("# HELP pistreaming_play_sessions Sesiones de reproducción activas.\n");
        out.push_str("# TYPE pistreaming_play_sessions gauge\n");
        out.push_str(&format!("pistreaming_play_sessions {sessions}\n"));
        out.push_str("# HELP pistreaming_torrents Handles de torrent vivos.\n");
        out.push_str("# TYPE pistreaming_torrents gauge\n");
        out.push_str(&format!("pistreaming_torrents {torrents}\n"));
        out.push_str("# HELP pistreaming_ffmpeg Procesos ffmpeg activos.\n");
        out.push_str("# TYPE pistreaming_ffmpeg gauge\n");
        out.push_str(&format!("pistreaming_ffmpeg {ffmpeg}\n"));
        out.push_str("# HELP pistreaming_cache_bytes Bytes en la caché de torrents.\n");
        out.push_str("# TYPE pistreaming_cache_bytes gauge\n");
        out.push_str(&format!("pistreaming_cache_bytes {cache}\n"));
        out.push_str("# HELP pistreaming_uptime_seconds Uptime del proceso.\n");
        out.push_str("# TYPE pistreaming_uptime_seconds gauge\n");
        out.push_str(&format!("pistreaming_uptime_seconds {uptime}\n"));
        out.push_str("# HELP pistreaming_build_info Información de build.\n");
        out.push_str("# TYPE pistreaming_build_info gauge\n");
        out.push_str("pistreaming_build_info{version=\"0.1.0\"} 1\n");
        out
    }
}

/// Middleware que cuenta peticiones por ruta (patrón), método y status.
pub async fn middleware(State(st): State<SharedState>, req: Request, next: Next) -> Response {
    let method = req.method().to_string();
    let path = req
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    let resp = next.run(req).await;
    st.metrics.record(&method, &path, resp.status().as_u16());
    resp
}

/// GET /metrics — formato de texto Prometheus.
pub async fn endpoint(State(st): State<SharedState>) -> Response {
    let body = st.metrics.render(&st);
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    #[test]
    fn render_incluye_gauges_y_contadores() {
        let dir = tempfile::tempdir().unwrap();
        let st = crate::test_state(dir.path().to_path_buf());
        st.metrics.record("GET", "/api/search", 200);
        st.metrics.record("GET", "/api/search", 200);
        st.metrics.record("POST", "/api/play", 500);
        let body = st.metrics.render(&st);
        assert!(body.contains("pistreaming_requests_total{method=\"GET\",path=\"/api/search\",status=\"200\"} 2"));
        assert!(body.contains("pistreaming_play_sessions 0"));
        assert!(body.contains("pistreaming_build_info"));
    }
}
