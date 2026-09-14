//! Normalización de URLs de addons.

/// Trim de espacios, recorte de `/` finales y del sufijo `/manifest.json`.
///
/// Es la única fuente de verdad para comparar/persistir URLs de addons:
/// evita duplicados por barra final y que un DELETE no encuentre la fila.
/// También garantiza que la URL quede como *base* del addon: si el usuario
/// pega la URL del manifest (`.../manifest.json`), se le quita el sufijo para
/// que `fetch_manifest`/`catalog`/`meta`/`streams` no dupliquen la ruta.
pub fn normalize_url(url: &str) -> String {
    let mut url = url.trim().trim_end_matches('/').to_string();
    while let Some(stripped) = url.strip_suffix("/manifest.json") {
        url = stripped.trim_end_matches('/').to_string();
    }
    url
}

#[cfg(test)]
mod tests {
    use super::normalize_url;

    #[test]
    fn trims_spaces_and_trailing_slashes() {
        assert_eq!(normalize_url("  http://a.test  "), "http://a.test");
        assert_eq!(normalize_url("http://a.test/"), "http://a.test");
        assert_eq!(normalize_url("http://a.test///"), "http://a.test");
        assert_eq!(normalize_url("  http://a.test/  "), "http://a.test");
        assert_eq!(normalize_url("http://a.test"), "http://a.test");
    }

    #[test]
    fn strips_manifest_suffix() {
        assert_eq!(normalize_url("https://x.test/manifest.json"), "https://x.test");
        assert_eq!(normalize_url("https://x.test/manifest.json/"), "https://x.test");
        assert_eq!(
            normalize_url("https://x.test/base/manifest.json"),
            "https://x.test/base"
        );
    }

    #[test]
    fn strips_manifest_suffix_idempotently() {
        assert_eq!(
            normalize_url("https://x.test/manifest.json/manifest.json"),
            "https://x.test"
        );
    }
}
