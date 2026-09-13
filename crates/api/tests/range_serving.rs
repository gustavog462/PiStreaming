//! Verifica que un reader seekable se sirve 200/206 vía ranged_response.
use axum::body::to_bytes;
use axum::http::{HeaderMap, HeaderValue};

#[tokio::test]
async fn sirve_206_con_rango() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f.bin");
    std::fs::write(&path, b"0123456789").unwrap();

    let file = tokio::fs::File::open(&path).await.unwrap();
    let meta = file.metadata().await.unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("range", HeaderValue::from_static("bytes=2-4"));

    let resp =
        pistreaming_api::range::ranged_response(file, meta.len(), &headers, "text/plain").await;
    assert_eq!(resp.status(), 206);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&body[..], b"234");
}
