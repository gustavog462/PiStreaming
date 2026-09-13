//! Normalización de URLs de addons.

/// Trim de espacios y recorte de `/` finales.
///
/// Es la única fuente de verdad para comparar/persistir URLs de addons:
/// evita duplicados por barra final y que un DELETE no encuentre la fila.
pub fn normalize_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
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
}
