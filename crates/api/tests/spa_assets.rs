//! La SPA embebida se sirve con su Content-Type (spec §8).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn index_sirve_el_shell_de_la_spa() {
    let tmp = tempfile::tempdir().unwrap();
    let app = pistreaming_api::router(pistreaming_api::test_state(tmp.path().to_path_buf()));
    let res = app
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let ct = res.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(ct.starts_with("text/html"), "content-type html: {ct}");
    let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("id=\"app\""), "shell con #app: {html}");
    assert!(html.contains("/assets/app.js"), "shell enlaza app.js: {html}");
}

#[tokio::test]
async fn assets_sirven_css_con_su_content_type() {
    let tmp = tempfile::tempdir().unwrap();
    let app = pistreaming_api::router(pistreaming_api::test_state(tmp.path().to_path_buf()));
    let res = app
        .clone()
        .oneshot(Request::get("/assets/style.css").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("text/css"));

    let res = app
        .oneshot(Request::get("/icon.svg").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("image/svg+xml"));
}

#[tokio::test]
async fn asset_inexistente_es_404() {
    let tmp = tempfile::tempdir().unwrap();
    let app = pistreaming_api::router(pistreaming_api::test_state(tmp.path().to_path_buf()));
    let res = app
        .oneshot(Request::get("/assets/nope.xyz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
