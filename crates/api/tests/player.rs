use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn player_sirve_html() {
    let tmp = tempfile::tempdir().unwrap();
    let app = pistreaming_api::router(pistreaming_api::test_state(tmp.path().to_path_buf()));
    let res = app
        .oneshot(Request::builder().uri("/play/abc").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("<video"));
}
