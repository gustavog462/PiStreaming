//! Tests de integración de la fase 0 (paridad con el binario desplegado):
//! pistas en el plan, cambio de audio, subtítulos externos sin API key, warm
//! sin engine y HLS sobre una fuente directa.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pistreaming_api::session::PlaySession;
use pistreaming_core::playback::{PlaybackPlan, PlaybackRoute, TrackInfo};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;
use tower::ServiceExt;

fn plan_con_pistas(session: &str, route: PlaybackRoute) -> PlaybackPlan {
    PlaybackPlan {
        session: session.into(),
        route,
        playback_url: format!("/stream/{session}"),
        raw_url: Some(format!("/raw/{session}")),
        browser_may_fail: false,
        needs_recode_audio: false,
        video_codec: "h264".into(),
        audio_codec: Some("aac".into()),
        audio_tracks: vec![
            TrackInfo {
                index: 1,
                codec: Some("aac".into()),
                language: Some("eng".into()),
                title: None,
                default: false,
                forced: false,
            },
            TrackInfo {
                index: 2,
                codec: Some("aac".into()),
                language: Some("spa".into()),
                title: Some("Español".into()),
                default: true,
                forced: false,
            },
        ],
        subtitle_tracks: vec![TrackInfo {
            index: 3,
            codec: Some("subrip".into()),
            language: Some("spa".into()),
            title: None,
            default: false,
            forced: false,
        }],
        active_audio_index: Some(1),
        progress_url: None,
    }
}

fn insert_session(
    st: &pistreaming_api::SharedState,
    id: &str,
    route: PlaybackRoute,
    dir: &std::path::Path,
) {
    st.sessions.insert(PlaySession {
        id: id.into(),
        info_hash: "abc".into(),
        file_id: 0,
        plan: plan_con_pistas(id, route),
        cache_dir: dir.to_path_buf(),
        created_at: Instant::now(),
        ffmpeg: None,
        progress_key: None,
        kind: Some("movie".into()),
        meta_id: Some("tt1".into()),
        title: Some("Peli".into()),
        media_path: None,
        magnet: None,
        transcode_done: Arc::new(AtomicBool::new(false)),
        audio_index: Some(2),
    });
}

#[tokio::test]
async fn plan_expone_pistas_y_audio_activo() {
    let tmp = tempfile::tempdir().unwrap();
    let st = pistreaming_api::test_state(tmp.path().to_path_buf());
    insert_session(&st, "s1", PlaybackRoute::Remux, tmp.path());
    let app = pistreaming_api::router(st);

    let res = app
        .oneshot(Request::get("/api/play/s1").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["audio_tracks"].as_array().unwrap().len(), 2);
    assert_eq!(v["subtitle_tracks"].as_array().unwrap().len(), 1);
    assert_eq!(v["active_audio_index"], 1);
    assert_eq!(v["audio_tracks"][1]["language"], "spa");
}

#[tokio::test]
async fn switch_audio_en_direct_es_422() {
    let tmp = tempfile::tempdir().unwrap();
    let st = pistreaming_api::test_state(tmp.path().to_path_buf());
    insert_session(&st, "s1", PlaybackRoute::Direct, tmp.path());
    let app = pistreaming_api::router(st);

    let res = app
        .oneshot(
            Request::post("/api/play/s1/audio")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"index":0}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn switch_audio_indice_invalido_es_400() {
    let tmp = tempfile::tempdir().unwrap();
    let st = pistreaming_api::test_state(tmp.path().to_path_buf());
    insert_session(&st, "s1", PlaybackRoute::Remux, tmp.path());
    let app = pistreaming_api::router(st);

    let res = app
        .oneshot(
            Request::post("/api/play/s1/audio")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"index":99}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn subtitulos_externos_sin_api_key_devuelven_lista_vacia() {
    let tmp = tempfile::tempdir().unwrap();
    let st = pistreaming_api::test_state(tmp.path().to_path_buf());
    let app = pistreaming_api::router(st);

    let res = app
        .oneshot(
            Request::get("/api/subtitles/external/movie/tt0133093")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["languages"], serde_json::json!([]));
}

#[tokio::test]
async fn warm_sin_engine_es_503() {
    let tmp = tempfile::tempdir().unwrap();
    let st = pistreaming_api::test_state(tmp.path().to_path_buf());
    let app = pistreaming_api::router(st);

    let res = app
        .oneshot(
            Request::post("/api/warm")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"info_hash":"abc"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn hls_sobre_direct_es_422() {
    let tmp = tempfile::tempdir().unwrap();
    let st = pistreaming_api::test_state(tmp.path().to_path_buf());
    insert_session(&st, "s1", PlaybackRoute::Direct, tmp.path());
    let app = pistreaming_api::router(st);

    let res = app
        .oneshot(
            Request::get("/hls/s1/index.m3u8")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
