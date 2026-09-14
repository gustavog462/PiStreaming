//! API de la biblioteca: `GET|POST|DELETE /api/library` y `GET /library/:id/stream`.
//!
//! Como `POST /api/library` exige un torrent descargado (handle vivo + `stats().finished`),
//! la ruta feliz se siembra con `Store::keep` sobre un archivo real y se verifica el
//! contrato HTTP; el chequeo de completitud se cubre con los casos 404/409.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use pistreaming_api::session::PlaySession;
use pistreaming_core::playback::{PlaybackPlan, PlaybackRoute};
use std::time::Instant;
use tower::ServiceExt;

fn plan(session: &str) -> PlaybackPlan {
    PlaybackPlan {
        session: session.into(),
        route: PlaybackRoute::Direct,
        playback_url: format!("/stream/{session}"),
        raw_url: Some(format!("/raw/{session}")),
        browser_may_fail: false,
        needs_recode_audio: false,
        video_codec: "h264".into(),
        audio_codec: Some("aac".into()),
        progress_url: None,
    }
}

fn seed_library(
    st: &pistreaming_api::SharedState,
    kind: &str,
    id: &str,
    bytes: &[u8],
) -> String {
    let src = st.cache_dir.join(format!("{kind}_{id}.mkv"));
    std::fs::write(&src, bytes).unwrap();
    st.store
        .keep(&st.library_dir, kind, id, "Título", Some("hashZ"), &src)
        .unwrap()
        .file_path
}

#[tokio::test]
async fn get_library_lista_y_delete_es_idempotente() {
    let tmp = tempfile::tempdir().unwrap();
    let st = pistreaming_api::test_state(tmp.path().to_path_buf());
    let file_path = seed_library(&st, "movie", "tt1", b"0123456789");
    let app = pistreaming_api::router(st.clone());

    let res = app
        .clone()
        .oneshot(Request::get("/api/library").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["items"][0]["id"], "movie:tt1");
    assert_eq!(v["items"][0]["size_bytes"], 10);

    let res = app
        .clone()
        .oneshot(
            Request::delete("/api/library/movie:tt1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    assert!(!std::path::Path::new(&file_path).exists(), "borra el archivo");

    // Idempotente: repetir sigue dando 204.
    let res = app
        .clone()
        .oneshot(
            Request::delete("/api/library/movie:tt1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    let res = app
        .oneshot(Request::get("/api/library").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["items"], serde_json::json!([]));
}

#[tokio::test]
async fn post_library_sin_sesion_es_404() {
    let tmp = tempfile::tempdir().unwrap();
    let app = pistreaming_api::router(pistreaming_api::test_state(tmp.path().to_path_buf()));
    let res = app
        .oneshot(
            Request::post("/api/library")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"session":"no-existe"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn post_library_con_descarga_incompleta_es_409() {
    let tmp = tempfile::tempdir().unwrap();
    let st = pistreaming_api::test_state(tmp.path().to_path_buf());
    let media = tmp.path().join("c.mkv");
    std::fs::write(&media, b"x").unwrap();
    st.sessions.insert(PlaySession {
        id: "sess-0".into(),
        info_hash: "hash".into(),
        file_id: 0,
        plan: plan("sess-0"),
        cache_dir: tmp.path().to_path_buf(),
        created_at: Instant::now(),
        ffmpeg: None,
        progress_key: Some("movie:tt9".into()),
        kind: Some("movie".into()),
        meta_id: Some("tt9".into()),
        title: Some("Peli".into()),
        media_path: Some(media),
    });
    let app = pistreaming_api::router(st);

    let res = app
        .oneshot(
            Request::post("/api/library")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"session":"sess-0"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::CONFLICT,
        "sin handle vivo => descarga incompleta"
    );
}

/// `POST /api/library` feliz: con un handle vivo y completo (`stats().finished`)
/// debe promover el archivo a `library_dir` y devolver la ficha con la identidad
/// de meta (`kind`/`meta_id` -> `id`) y el `title` de la `PlaySession`.
///
/// Se siembra un torrent local con `create_and_serve_torrent`: al tener todos los
/// fragmentos presentes, `stats().finished` es true sin depender de la red.
#[tokio::test]
async fn post_library_keep_con_handle_completo_guarda_en_library_dir() {
    use librqbit::{CreateTorrentOptions, Session, SessionOptions};

    let tmp = tempfile::tempdir().unwrap();
    let st = pistreaming_api::test_state(tmp.path().to_path_buf());

    // Sesión local aislada (sin DHT ni trackers) que siembra un archivo ya presente.
    let seeder: std::sync::Arc<Session> = Session::new_with_opts(
        tmp.path().to_path_buf(),
        SessionOptions {
            dht: None,
            disable_trackers: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let media = tmp.path().join("ttK.mkv");
    std::fs::write(&media, b"0123456789").unwrap();
    let (_meta, handle) = seeder
        .create_and_serve_torrent(&media, CreateTorrentOptions::default())
        .await
        .unwrap();
    handle.wait_until_initialized().await.unwrap();

    // El archivo ya está completo en disco; la verificación marca el torrent.
    let mut finished = false;
    for _ in 0..100 {
        if handle.stats().finished {
            finished = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(
        finished,
        "el torrent sembrado localmente debe quedar completo (stats().finished)"
    );

    let session_id = "sess-keep";
    st.handles
        .write()
        .insert(session_id.to_string(), handle.clone());
    st.sessions.insert(PlaySession {
        id: session_id.into(),
        info_hash: handle.info_hash().as_string(),
        file_id: 0,
        plan: plan(session_id),
        cache_dir: tmp.path().to_path_buf(),
        created_at: Instant::now(),
        ffmpeg: None,
        progress_key: Some("movie:ttK".into()),
        kind: Some("movie".into()),
        meta_id: Some("ttK".into()),
        title: Some("Mi Peli".into()),
        media_path: Some(media.clone()),
    });

    let app = pistreaming_api::router(st.clone());
    let res = app
        .oneshot(
            Request::post("/api/library")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"session":"sess-keep"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        res.status(),
        StatusCode::CREATED,
        "keep con handle completo debe crear la ficha"
    );
    let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["id"], "movie:ttK", "el id es `kind:meta_id`");
    assert_eq!(v["kind"], "movie");
    assert_eq!(v["title"], "Mi Peli", "recupera el title de la PlaySession");

    // (b) el archivo quedó efectivamente en `library_dir`, con el contenido real.
    let saved = st.library_dir.join("movie:ttK.mkv");
    assert!(
        saved.exists(),
        "el archivo debe quedar en library_dir: {}",
        saved.display()
    );
    assert_eq!(std::fs::read(&saved).unwrap(), b"0123456789");
}

#[tokio::test]
async fn library_stream_sirve_rango_206() {
    let tmp = tempfile::tempdir().unwrap();
    let st = pistreaming_api::test_state(tmp.path().to_path_buf());
    seed_library(&st, "movie", "tt2", b"0123456789");
    let app = pistreaming_api::router(st);

    let res = app
        .oneshot(
            Request::get("/library/movie:tt2/stream")
                .header("range", "bytes=2-4")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&body[..], b"234");
}

#[tokio::test]
async fn library_stream_id_inexistente_es_404() {
    let tmp = tempfile::tempdir().unwrap();
    let app = pistreaming_api::router(pistreaming_api::test_state(tmp.path().to_path_buf()));
    let res = app
        .oneshot(
            Request::get("/library/movie:nope/stream")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
