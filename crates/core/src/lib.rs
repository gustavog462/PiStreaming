//! Modelos y helpers puros del protocolo de addons de Stremio. Sin I/O.
pub mod catalog;
pub mod error;
pub mod manifest;
pub mod meta;
pub mod playback;
pub mod stream;
pub mod url;

pub use error::{CoreError, CoreResult};
pub use url::normalize_url;
