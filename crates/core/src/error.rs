use thiserror::Error;

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("http error: {0}")]
    Http(String),
    #[error("json error: {0}")]
    Json(String),
    #[error("db error: {0}")]
    Db(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("addon error: {0}")]
    Addon(String),
    #[error("no soportado: {0}")]
    Unsupported(String),
    #[error("{0}")]
    Other(String),
}

impl From<serde_json::Error> for CoreError {
    fn from(e: serde_json::Error) -> Self {
        CoreError::Json(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_lleva_mensaje() {
        let e = CoreError::Unsupported("video codec no soportado: mpeg2video".into());
        assert_eq!(e.to_string(), "no soportado: video codec no soportado: mpeg2video");
    }
}
