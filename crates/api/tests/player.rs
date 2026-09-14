use axum::http::{Request, StatusCode};
use tower::ServiceExt;

/// `/play/:session` ya no sirve HTML propio: redirige a la SPA (`#/player/:session`).
#[tokio::test]
async fn play_redirige_a_la_spa() {
    let tmp = tempfile::tempdir().unwrap();
    let app = pistreaming_api::router(pistreaming_api::test_state(tmp.path().to_path_buf()));
    let res = app
        .oneshot(
            Request::builder()
                .uri("/play/abc-0")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        res.headers().get("location").unwrap(),
        "/#/player/abc-0",
        "debe apuntar al hash-routing de la SPA"
    );
}
