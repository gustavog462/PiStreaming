use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn put_luego_get_progreso() {
    let tmp = tempfile::tempdir().unwrap();
    let st = pistreaming_api::test_state(tmp.path().to_path_buf());
    let app = pistreaming_api::router(st.clone());

    let put = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/progress/movie/tt123")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"position":42.5,"duration":100.0}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::NO_CONTENT);

    let get = app
        .oneshot(Request::builder().uri("/api/progress/movie/tt123").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::OK);
    let body = axum::body::to_bytes(get.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["position"], 42.5);
    assert_eq!(v["duration"], 100.0);
}
