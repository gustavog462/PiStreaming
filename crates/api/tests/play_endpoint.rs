//! Verifica POST /api/play cuando el engine de torrents todavía no fue creado.
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn play_sin_engine_da_503() {
    let tmp = tempfile::tempdir().unwrap();
    let st = pistreaming_api::test_state(tmp.path().to_path_buf());
    let app = pistreaming_api::router(st);
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/play")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"magnet":"magnet:?xt=urn:btih:0000000000000000000000000000000000000000"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}
