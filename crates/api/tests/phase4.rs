//! Tests de la fase 4: Discover (catálogo), Continuar viendo, métricas y auth.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn continue_devuelve_solo_progreso_en_curso() {
    let tmp = tempfile::tempdir().unwrap();
    let st = pistreaming_api::test_state(tmp.path().to_path_buf());
    st.store
        .save_progress("movie:tt1", 100.0, Some(1000.0))
        .unwrap();
    // Casi terminado: no debe aparecer.
    st.store
        .save_progress("movie:tt2", 990.0, Some(1000.0))
        .unwrap();
    let app = pistreaming_api::router(st);
    let res = app
        .oneshot(Request::get("/api/continue").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["items"].as_array().unwrap().len(), 1);
    assert_eq!(v["items"][0]["id"], "tt1");
    assert_eq!(v["items"][0]["kind"], "movie");
}

#[tokio::test]
async fn catalog_sin_addons_devuelve_vacio() {
    let tmp = tempfile::tempdir().unwrap();
    let st = pistreaming_api::test_state(tmp.path().to_path_buf());
    let app = pistreaming_api::router(st);
    let res = app
        .oneshot(
            Request::get("/api/catalog/movie/top")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["metas"], serde_json::json!([]));
}

#[tokio::test]
async fn metrics_expone_gauges() {
    let tmp = tempfile::tempdir().unwrap();
    let st = pistreaming_api::test_state(tmp.path().to_path_buf());
    let app = pistreaming_api::router(st);
    let res = app
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8_lossy(&bytes);
    assert!(body.contains("pistreaming_play_sessions"));
    assert!(body.contains("pistreaming_build_info"));
}

#[tokio::test]
async fn auth_protege_api_y_acepta_api_key_y_cookie() {
    let tmp = tempfile::tempdir().unwrap();
    let auth = Arc::new(pistreaming_api::auth::AuthConfig::new_for_test(
        Some("pw".into()),
        b"secret".to_vec(),
        Some("key".into()),
    ));
    let st = pistreaming_api::test_state_with_auth(tmp.path().to_path_buf(), auth);
    let app = pistreaming_api::router(st);

    // Sin credencial -> 401.
    let res = app
        .clone()
        .oneshot(
            Request::get("/api/search?query=x")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // API key -> 200.
    let res = app
        .clone()
        .oneshot(
            Request::get("/api/search?query=x")
                .header("authorization", "Bearer key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Login -> cookie.
    let res = app
        .clone()
        .oneshot(
            Request::post("/api/login")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("password=pw&next=/"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    let cookie = res
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    // Con cookie -> 200.
    let res = app
        .oneshot(
            Request::get("/api/search?query=x")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}
