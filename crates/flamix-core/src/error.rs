use thiserror::Error;

/// Error type of flamix-core.
#[derive(Debug, Error)]
pub enum Error {
    /// A share link or subscription body could not be parsed.
    #[error("parse error: {0}")]
    Parse(String),
    /// The link scheme / protocol is not supported.
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("http error: {0}")]
    Http(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
