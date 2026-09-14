//! El guardado de progreso del player depende de que `PlaybackPlan.progress_url`
//! viaje en el plan y de la ruta de dos segmentos `/api/progress/:kind/:id`.
//!
//! `POST /api/play` requiere el engine de torrents (503 sin él), así que acá se
//! registra la `PlaySession` a mano con el plan que `play()` habría construido, y
//! se verifica el contrato observable: `GET /api/play/:session` expone la URL y
//! el round-trip `PUT`/`GET` sobre esa URL persiste la posición.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use pistreaming_api::session::PlaySession;
use pistreaming_core::playback::{PlaybackPlan, PlaybackRoute};
use std::time::Instant;
use tower::ServiceExt;

fn plan_con_progress_url(session: &str, progress_url: Option<String>) -> PlaybackPlan {
    PlaybackPlan {
        session: session.into(),
        route: PlaybackRoute::Direct,
        playback_url: format!("http://127.0.0.1:8000/stream/{session}"),
        raw_url: Some(format!("http://127.0.0.1:8000/raw/{session}")),
        browser_may_fail: false,
        needs_recode_audio: false,
        video_codec: "h264".into(),
        audio_codec: Some("aac".into()),
        progress_url,
    }
}

/// `GET /api/play/:session` incluye `progress_url` con la ruta de dos segmentos.
#[tokio::test]
async fn plan_expone_progress_url_de_dos_segmentos() {
    let tmp = tempfile::tempdir().unwrap();
    let st = pistreaming_api::test_state(tmp.path().to_path_buf());
    let session = "dd8255ecdc7c-1";
    st.sessions.insert(PlaySession {
        id: session.into(),
        info_hash: "dd8255ecdc7c".into(),
        file_id: 1,
        plan: plan_con_progress_url(
            session,
            Some("http://127.0.0.1:8000/api/progress/movie/tt123".into()),
        ),
        cache_dir: tmp.path().to_path_buf(),
        created_at: Instant::now(),
        ffmpeg: None,
        progress_key: Some("movie:tt123".into()),
        kind: Some("movie".into()),
        meta_id: Some("tt123".into()),
        title: Some("Dune".into()),
        media_path: None,
    });

    let app = pistreaming_api::router(st);
    let res = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/play/{session}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        v["progress_url"], "http://127.0.0.1:8000/api/progress/movie/tt123",
        "el plan debe exponer la URL de progreso: {v}"
    );
}

/// El `PUT`/`GET` sobre la URL de dos segmentos persiste la posición.
#[tokio::test]
async fn progress_roundtrip_dos_segmentos() {
    let tmp = tempfile::tempdir().unwrap();
    let app = pistreaming_api::router(pistreaming_api::test_state(tmp.path().to_path_buf()));

    let put = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/progress/movie/dd8255ecdc7c-1")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"position":42.5,"duration":634.6}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::NO_CONTENT);

    let get = app
        .oneshot(
            Request::builder()
                .uri("/api/progress/movie/dd8255ecdc7c-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::OK);
    let body = axum::body::to_bytes(get.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["position"], 42.5);
    assert_eq!(v["duration"], 634.6);
}
