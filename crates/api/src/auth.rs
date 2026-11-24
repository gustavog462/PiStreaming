//! Autenticación: contraseña + cookie firmada, API key y token de embed.
//!
//! Se activa con `AUTH_PASSWORD` en el entorno. Sin esa variable la app queda
//! abierta (comportamiento actual). `AUTH_SECRET` firma la cookie y los tokens
//! de embed (si falta, se deriva de la contraseña). `PISTREAMING_API_KEY` es
//! opcional para clientes externos (`Authorization: Bearer` o `?api_key=`).

use std::sync::Arc;

use axum::extract::{Form, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::Json;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::SharedState;

const COOKIE: &str = "pistreaming_session";
const COOKIE_MAX_AGE: i64 = 30 * 24 * 3600;

type HmacSha256 = Hmac<Sha256>;

fn sign(secret: &[u8], msg: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC acepta cualquier clave");
    mac.update(msg);
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix(&format!("{name}=")) {
            return Some(v.to_string());
        }
    }
    None
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Configuración de autenticación (inmutable tras el arranque).
pub struct AuthConfig {
    password: Option<String>,
    secret: Vec<u8>,
    api_key: Option<String>,
}

impl AuthConfig {
    pub fn from_env() -> Self {
        let password = std::env::var("AUTH_PASSWORD")
            .ok()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty());
        let secret = std::env::var("AUTH_SECRET")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(String::into_bytes)
            .unwrap_or_else(|| {
                let mut h = Sha256::new();
                h.update(password.as_deref().unwrap_or("").as_bytes());
                h.update(b"pistreaming-auth-v1");
                h.finalize().to_vec()
            });
        let api_key = std::env::var("PISTREAMING_API_KEY")
            .ok()
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty());
        Self {
            password,
            secret,
            api_key,
        }
    }

    #[doc(hidden)]
    pub fn new_for_test(
        password: Option<String>,
        secret: Vec<u8>,
        api_key: Option<String>,
    ) -> Self {
        Self {
            password,
            secret,
            api_key,
        }
    }

    pub fn enabled(&self) -> bool {
        self.password.is_some()
    }

    fn check_password(&self, input: &str) -> bool {
        match self.password.as_deref() {
            Some(p) => ct_eq(input.as_bytes(), p.as_bytes()),
            None => true,
        }
    }

    fn sign_msg(&self, msg: &str) -> String {
        sign(&self.secret, msg.as_bytes())
    }

    fn cookie_value(&self) -> String {
        let exp = unix_now() + COOKIE_MAX_AGE;
        format!("{exp}.{}", self.sign_msg(&exp.to_string()))
    }

    fn valid_cookie(&self, value: &str) -> bool {
        let Some((exp, sig)) = value.split_once('.') else {
            return false;
        };
        let Ok(exp) = exp.parse::<i64>() else {
            return false;
        };
        if exp < unix_now() {
            return false;
        }
        ct_eq(sig.as_bytes(), self.sign_msg(&exp.to_string()).as_bytes())
    }

    /// Token de embed para una sesión (HMAC del id).
    pub fn embed_token(&self, session: &str) -> String {
        self.sign_msg(&format!("embed:{session}"))
    }

    /// ¿La petición está autorizada? (cookie, API key o token de embed).
    pub fn check(&self, headers: &HeaderMap, query: Option<&str>, session: Option<&str>) -> bool {
        if !self.enabled() {
            return true;
        }
        if let Some(c) = cookie_value(headers, COOKIE) {
            if self.valid_cookie(&c) {
                return true;
            }
        }
        if let Some(key) = self.api_key.as_deref() {
            if let Some(auth) = headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
            {
                if let Some(t) = auth.strip_prefix("Bearer ") {
                    if ct_eq(t.as_bytes(), key.as_bytes()) {
                        return true;
                    }
                }
            }
            if let Some(q) = query {
                for kv in q.split('&') {
                    if let Some(v) = kv.strip_prefix("api_key=") {
                        if ct_eq(v.as_bytes(), key.as_bytes()) {
                            return true;
                        }
                    }
                }
            }
        }
        if let Some(sid) = session {
            if let Some(q) = query {
                for kv in q.split('&') {
                    if let Some(v) = kv.strip_prefix("token=") {
                        if ct_eq(v.as_bytes(), self.embed_token(sid).as_bytes()) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

fn is_public(path: &str) -> bool {
    path == "/login"
        || path == "/api/login"
        || path == "/api/health"
        || path == "/embed.js"
        || path == "/metrics"
        || path == "/icon.svg"
        || path.starts_with("/assets/")
        || path.starts_with("/embed/")
}

/// Extrae el id de sesión de las rutas que sirven media (para el token de embed).
fn session_from_path(path: &str) -> Option<String> {
    let rest = path
        .strip_prefix("/stream/")
        .or_else(|| path.strip_prefix("/raw/"))
        .or_else(|| path.strip_prefix("/hls/"))
        .or_else(|| path.strip_prefix("/thumb/"))
        .or_else(|| path.strip_prefix("/embed/"))
        .or_else(|| path.strip_prefix("/api/zip/"))?;
    let sid = rest.split('/').next().unwrap_or("");
    if sid.is_empty() {
        None
    } else {
        Some(sid.to_string())
    }
}

/// Middleware de autenticación.
pub async fn middleware(State(st): State<SharedState>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    if is_public(&path) {
        return next.run(req).await;
    }
    let query = req.uri().query().map(str::to_string);
    let session = session_from_path(&path);
    if st
        .auth
        .check(req.headers(), query.as_deref(), session.as_deref())
    {
        return next.run(req).await;
    }
    // API y media: 401 (un redirect a /login rompería <video>/fetch).
    if path.starts_with("/api/")
        || path.starts_with("/stream/")
        || path.starts_with("/raw/")
        || path.starts_with("/hls/")
        || path.starts_with("/thumb/")
    {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "no autorizado", "code": 401 })),
        )
            .into_response();
    }
    let next_url = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/");
    Redirect::to(&format!("/login?next={}", urlencode(next_url))).into_response()
}

fn login_page_html(next: Option<&str>, error: bool) -> String {
    let next = next.unwrap_or("/");
    let next_attr = html_escape(next);
    let err = if error {
        r#"<p class="err">Contraseña incorrecta</p>"#
    } else {
        ""
    };
    format!(
        r#"<!doctype html>
<html lang="es">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>PiStreaming · Entrar</title>
<style>
:root{{color-scheme:dark}}
*{{box-sizing:border-box}}
body{{margin:0;min-height:100vh;display:grid;place-items:center;background:#0b0d12;color:#e8e8ea;font:16px/1.5 system-ui,sans-serif}}
form{{width:min(360px,92vw);background:#141821;border:1px solid #232a38;border-radius:14px;padding:28px}}
h1{{margin:0 0 6px;font-size:22px}}
p.sub{{margin:0 0 18px;color:#9aa3b2;font-size:14px}}
input[type=password]{{width:100%;padding:12px 14px;border-radius:10px;border:1px solid #2c3444;background:#0f131b;color:#fff;font-size:16px}}
button{{margin-top:14px;width:100%;padding:12px;border:0;border-radius:10px;background:#e11d48;color:#fff;font-size:16px;font-weight:600;cursor:pointer}}
.err{{color:#fb7185;font-size:14px;margin:10px 0 0}}
</style>
</head>
<body>
<form method="post" action="/api/login">
  <h1>Pi<span style="color:#e11d48">Streaming</span></h1>
  <p class="sub">Introduce la contraseña para entrar.</p>
  <input type="password" name="password" placeholder="Contraseña" autofocus autocomplete="current-password">
  <input type="hidden" name="next" value="{next_attr}">
  <button type="submit">Entrar</button>
  {err}
</form>
</body>
</html>"#
    )
}

/// GET /login — formulario de acceso.
pub async fn login_page(
    State(st): State<SharedState>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    if !st.auth.enabled() {
        return Redirect::to("/").into_response();
    }
    let next = q.get("next").map(String::as_str);
    Html(login_page_html(next, false)).into_response()
}

#[derive(Deserialize)]
pub struct LoginForm {
    password: String,
    #[serde(default)]
    next: Option<String>,
}

/// POST /api/login — valida la contraseña y pone la cookie de sesión.
pub async fn login(State(st): State<SharedState>, Form(form): Form<LoginForm>) -> Response {
    if !st.auth.enabled() {
        return Redirect::to("/").into_response();
    }
    if !st.auth.check_password(&form.password) {
        return (
            StatusCode::UNAUTHORIZED,
            Html(login_page_html(form.next.as_deref(), true)),
        )
            .into_response();
    }
    let cookie = format!(
        "{COOKIE}={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={COOKIE_MAX_AGE}",
        st.auth.cookie_value()
    );
    let next = form
        .next
        .as_deref()
        .filter(|n| n.starts_with('/'))
        .unwrap_or("/");
    let mut resp = Redirect::to(next).into_response();
    if let Ok(v) = HeaderValue::from_str(&cookie) {
        resp.headers_mut().insert(header::SET_COOKIE, v);
    }
    resp
}

/// POST /api/logout — borra la cookie y vuelve al login.
pub async fn logout() -> Response {
    let cookie = format!("{COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    let mut resp = Redirect::to("/login").into_response();
    if let Ok(v) = HeaderValue::from_str(&cookie) {
        resp.headers_mut().insert(header::SET_COOKIE, v);
    }
    resp
}

/// `Arc<AuthConfig>` para el estado.
pub type SharedAuth = Arc<AuthConfig>;

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> AuthConfig {
        AuthConfig {
            password: Some("secreto".into()),
            secret: b"test-secret".to_vec(),
            api_key: Some("api-key-123".into()),
        }
    }

    #[test]
    fn cookie_valida_y_expira() {
        let c = cfg();
        let v = c.cookie_value();
        assert!(c.valid_cookie(&v));
        assert!(!c.valid_cookie("0.abc"));
        assert!(!c.valid_cookie("basura"));
    }

    #[test]
    fn check_cookie_api_key_y_token() {
        let c = cfg();
        let mut h = HeaderMap::new();
        assert!(!c.check(&h, None, None));
        h.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{COOKIE}={}", c.cookie_value())).unwrap(),
        );
        assert!(c.check(&h, None, None));
        let mut h2 = HeaderMap::new();
        h2.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer api-key-123"));
        assert!(c.check(&h2, None, None));
        let tok = c.embed_token("sess-1");
        assert!(c.check(&HeaderMap::new(), Some(&format!("token={tok}")), Some("sess-1")));
        assert!(!c.check(&HeaderMap::new(), Some("token=malo"), Some("sess-1")));
    }

    #[test]
    fn sin_password_esta_abierto() {
        let c = AuthConfig {
            password: None,
            secret: b"x".to_vec(),
            api_key: None,
        };
        assert!(!c.enabled());
        assert!(c.check(&HeaderMap::new(), None, None));
    }

    #[test]
    fn sesion_desde_path() {
        assert_eq!(session_from_path("/stream/abc-1"), Some("abc-1".into()));
        assert_eq!(session_from_path("/raw/abc-1"), Some("abc-1".into()));
        assert_eq!(session_from_path("/hls/abc-1/index.m3u8"), Some("abc-1".into()));
        assert_eq!(session_from_path("/api/zip/abc-1"), Some("abc-1".into()));
        assert_eq!(session_from_path("/api/search"), None);
    }
}
