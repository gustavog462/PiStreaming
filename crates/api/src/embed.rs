//! Embed: página mínima del reproductor y SDK JS para incrustarlo.

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use pistreaming_core::playback::PlaybackRoute;

use crate::recovery;
use crate::SharedState;

/// GET /embed/:session — página con el `<video>` apuntando al stream de la sesión.
pub async fn page(State(st): State<SharedState>, Path(session): Path<String>) -> Response {
    let sess = match recovery::ensure_session(&st, &session).await {
        Ok(s) => s,
        Err(e) => return crate::err(StatusCode::NOT_FOUND, &e),
    };
    let (route, file_id) = {
        let g = sess.read();
        (g.plan.route, g.file_id)
    };
    // Token de embed: el iframe es cross-site y no lleva la cookie de sesión.
    let token = st.auth.embed_token(&session);
    let src = match route {
        PlaybackRoute::Direct => {
            format!("/raw/{}?file_id={}&token={}", session, file_id, token)
        }
        _ => format!("/stream/{}?token={}", session, token),
    };
    let html = format!(
        r#"<!doctype html>
<html lang="es">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>PiStreaming</title>
<style>html,body{{margin:0;height:100%;background:#000}}video{{width:100%;height:100%;object-fit:contain}}</style>
</head>
<body><video controls autoplay playsinline src="{src}"></video></body>
</html>"#
    );
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response()
}

/// GET /embed.js — SDK: convierte `[data-pistreaming-session]` en un iframe.
pub async fn sdk() -> Response {
    let js = r#"(function () {
  function mount(el) {
    var s = el.getAttribute('data-pistreaming-session');
    if (!s) return;
    var iframe = document.createElement('iframe');
    iframe.src = location.origin + '/embed/' + encodeURIComponent(s);
    iframe.style.width = el.getAttribute('data-width') || '100%';
    iframe.style.aspectRatio = '16/9';
    iframe.style.border = '0';
    iframe.setAttribute('allow', 'autoplay; fullscreen; picture-in-picture');
    iframe.setAttribute('allowfullscreen', '');
    el.replaceChildren(iframe);
  }
  function run() {
    document.querySelectorAll('[data-pistreaming-session]').forEach(mount);
  }
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', run);
  } else {
    run();
  }
})();"#;
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        js,
    )
        .into_response()
}
