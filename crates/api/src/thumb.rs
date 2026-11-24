//! Miniatura de un vídeo: frame extraído con ffmpeg y cacheado en la sesión.

use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::Response;

use crate::recovery;
use crate::SharedState;

const WAIT: Duration = Duration::from_secs(20);

/// GET /thumb/:session — JPEG de miniatura (cacheado en `cache/<hash>/thumb.jpg`).
pub async fn serve(State(st): State<SharedState>, Path(session): Path<String>) -> Response {
    let sess = match recovery::ensure_session(&st, &session).await {
        Ok(s) => s,
        Err(e) => return crate::err(StatusCode::NOT_FOUND, &e),
    };
    let (cache_dir, file_id) = {
        let g = sess.read();
        (g.cache_dir.clone(), g.file_id)
    };
    let path = cache_dir.join("thumb.jpg");
    if !path.exists() {
        let raw_url = format!("{}/raw/{}?file_id={}", st.public_base, session, file_id);
        let mut cmd = tokio::process::Command::new("ffmpeg");
        cmd.args([
            "-v", "error", "-y", "-ss", "1", "-rw_timeout", "15000000",
            "-i", &raw_url, "-frames:v", "1", "-vf", "scale=640:-1", "-q:v", "3",
        ])
        .arg(&path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
        match cmd.spawn() {
            Ok(mut child) => {
                let _ = tokio::time::timeout(WAIT, child.wait()).await;
                let _ = child.kill().await;
            }
            Err(e) => {
                return crate::core_err(pistreaming_core::error::CoreError::Other(format!(
                    "ffmpeg no ejecutable: {e}"
                )))
            }
        }
    }
    let mut waited = Duration::ZERO;
    while !path.exists() {
        if waited >= WAIT {
            return crate::err(StatusCode::NOT_FOUND, "miniatura no disponible");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        waited += Duration::from_millis(100);
    }
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let mut resp = Response::new(Body::from(bytes));
            resp.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("image/jpeg"),
            );
            resp.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=3600"),
            );
            resp
        }
        Err(e) => crate::err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("no se pudo leer la miniatura: {e}"),
        ),
    }
}
