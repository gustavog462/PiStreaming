//! Endpoints de la biblioteca permanente (spec §5.1).

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use pistreaming_core::error::CoreError;
use serde::Deserialize;

use crate::SharedState;

#[derive(Deserialize)]
pub struct KeepBody {
    pub session: String,
}

/// GET /api/library
pub async fn list(State(st): State<SharedState>) -> Response {
    match st.store.list_library() {
        Ok(items) => Json(serde_json::json!({ "items": items })).into_response(),
        Err(e) => crate::core_err(e),
    }
}

/// POST /api/library { session } -> 201 + ficha (dispara el `keep`).
pub async fn keep(
    State(st): State<SharedState>,
    body: Result<Json<KeepBody>, JsonRejection>,
) -> Response {
    let Json(body) = match body {
        Ok(b) => b,
        Err(_) => return crate::err(StatusCode::BAD_REQUEST, "body inválido"),
    };
    let Some(sess) = st.sessions.get(&body.session) else {
        return crate::err(StatusCode::NOT_FOUND, "sesión desconocida");
    };
    // Completitud: el torrent debe estar descargado (`stats().finished`).
    let handle = st.handles.read().get(&body.session).cloned();
    let finished = handle.map(|h| h.stats().finished).unwrap_or(false);
    if !finished {
        return crate::err(StatusCode::CONFLICT, "la descarga no está completa");
    }

    let (info_hash, kind, meta_id, title, media_path) = {
        let g = sess.read();
        (
            g.info_hash.clone(),
            g.kind.clone(),
            g.meta_id.clone(),
            g.title.clone(),
            g.media_path.clone(),
        )
    };
    let (Some(kind), Some(id), Some(media_path)) = (kind, meta_id, media_path) else {
        return crate::err(
            StatusCode::CONFLICT,
            "la sesión no tiene identidad de ficha (kind/id)",
        );
    };
    let title = title.unwrap_or_else(|| id.clone());
    let info_hash_opt = if info_hash.is_empty() { None } else { Some(info_hash) };

    match st
        .store
        .keep(&st.library_dir, &kind, &id, &title, info_hash_opt.as_deref(), &media_path)
    {
        Ok(item) => (StatusCode::CREATED, Json(item)).into_response(),
        Err(CoreError::NotFound(_)) => {
            crate::err(StatusCode::CONFLICT, "el archivo de la sesión no está disponible")
        }
        Err(e) => crate::core_err(e),
    }
}

/// DELETE /api/library/:id — idempotente.
pub async fn delete(State(st): State<SharedState>, Path(id): Path<String>) -> Response {
    let id = crate::decode_url(&id);
    match st.store.remove_library(&id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => crate::core_err(e),
    }
}

/// GET /library/:id/stream — sirve el archivo guardado con soporte de Range.
pub async fn stream(
    State(st): State<SharedState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let id = crate::decode_url(&id);
    let item = match st.store.list_library() {
        Ok(items) => items.into_iter().find(|i| i.id == id),
        Err(e) => return crate::core_err(e),
    };
    let Some(item) = item else {
        return crate::err(StatusCode::NOT_FOUND, "no está en la biblioteca");
    };
    let path = std::path::PathBuf::from(&item.file_path);
    let file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return crate::err(StatusCode::NOT_FOUND, "archivo no encontrado")
        }
        Err(e) => {
            return crate::core_err(CoreError::Other(format!(
                "no se pudo abrir {}: {e}",
                path.display()
            )))
        }
    };
    let len = match file.metadata().await {
        Ok(m) => m.len(),
        Err(e) => return crate::core_err(CoreError::Other(format!("no se pudo medir: {e}"))),
    };
    crate::range::ranged_response(file, len, &headers, mime_for_path(&path)).await
}

fn mime_for_path(p: &std::path::Path) -> &'static str {
    match p
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("mkv") => "video/x-matroska",
        Some("mp4") | Some("m4v") => "video/mp4",
        Some("webm") => "video/webm",
        Some("avi") => "video/x-msvideo",
        Some("mov") => "video/quicktime",
        Some("ts") => "video/mp2t",
        _ => "application/octet-stream",
    }
}
