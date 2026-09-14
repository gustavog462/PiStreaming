//! Assets estáticos de la SPA, embebidos con `rust-embed`.
//!
//! En debug lee del disco (`CARGO_MANIFEST_DIR/assets`), así editar `app.js` y
//! recargar no requiere recompilar; en release embebe los bytes en el binario.

use axum::body::Body;
use axum::extract::Path;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets/"]
struct Assets;

const INDEX: &str = "index.html";

/// GET / — shell de la SPA.
pub async fn index() -> Response {
    serve(INDEX).unwrap_or_else(|| {
        (StatusCode::INTERNAL_SERVER_ERROR, "index.html no embebido").into_response()
    })
}

/// GET /icon.svg — favicon.
pub async fn icon() -> Response {
    serve("icon.svg").unwrap_or_else(|| StatusCode::NOT_FOUND.into_response())
}

/// GET /assets/*path — `app.js` / `style.css` con su Content-Type.
pub async fn asset(Path(path): Path<String>) -> Response {
    serve(&path).unwrap_or_else(|| StatusCode::NOT_FOUND.into_response())
}

/// GET /play/:session — compatibilidad: redirige a la SPA con hash-routing.
pub async fn play_redirect(Path(session): Path<String>) -> Response {
    let location = format!("/#/player/{session}");
    (
        StatusCode::TEMPORARY_REDIRECT,
        [(
            header::LOCATION,
            HeaderValue::from_str(&location).unwrap_or(HeaderValue::from_static("/")),
        )],
    )
        .into_response()
}

fn serve(path: &str) -> Option<Response> {
    let file = Assets::get(path)?;
    let mime = mime_for(path);
    let mut resp = Response::new(Body::from(file.data.into_owned()));
    *resp.status_mut() = StatusCode::OK;
    resp.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache_control(path)));
    Some(resp)
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

fn cache_control(path: &str) -> &'static str {
    if path == INDEX {
        "no-cache"
    } else {
        "public, max-age=3600"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_e_icon_estan_embebidos() {
        assert!(Assets::get("index.html").is_some(), "falta crates/api/assets/index.html");
        assert!(Assets::get("icon.svg").is_some(), "falta crates/api/assets/icon.svg");
    }

    #[test]
    fn mime_por_extension() {
        assert_eq!(mime_for("index.html"), "text/html; charset=utf-8");
        assert_eq!(mime_for("app.js"), "text/javascript; charset=utf-8");
        assert_eq!(mime_for("style.css"), "text/css; charset=utf-8");
        assert_eq!(mime_for("icon.svg"), "image/svg+xml");
    }

    #[test]
    fn index_no_se_cachea() {
        assert_eq!(cache_control("index.html"), "no-cache");
        assert_eq!(cache_control("app.js"), "public, max-age=3600");
    }
}
