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
    #[error("{0}")]
    Other(String),
}

impl From<serde_json::Error> for CoreError {
    fn from(e: serde_json::Error) -> Self {
        CoreError::Json(e.to_string())
    }
}
