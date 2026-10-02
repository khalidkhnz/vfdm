use crate::types::DownloadId;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("invalid url: {0}")]
    InvalidUrl(String),
    #[error("http error: {}", chain(.0))]
    Http(#[from] reqwest::Error),
    #[error("server returned HTTP {0}")]
    Status(u16),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("remote file changed since download started")]
    RemoteChanged,
    #[error("server ignored range request")]
    RangeIgnored,
    #[error("server returned wrong content range")]
    RangeMismatch,
    #[error("cancelled")]
    Cancelled,
    #[error("download {0} not found")]
    NotFound(DownloadId),
    #[error("download {0} is {1}")]
    BadState(DownloadId, &'static str),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// reqwest's Display hides the cause ("error sending request"); walk the chain.
fn chain(e: &reqwest::Error) -> String {
    let mut parts = vec![e.to_string()];
    let mut src = std::error::Error::source(e);
    while let Some(s) = src {
        parts.push(s.to_string());
        src = s.source();
    }
    parts.dedup();
    parts.join(": ")
}
