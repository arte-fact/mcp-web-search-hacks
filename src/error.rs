use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("browser launch failed: {0}")]
    BrowserLaunch(#[source] anyhow::Error),

    #[error("navigation failed for {url}: {source}")]
    Navigation { url: String, source: anyhow::Error },

    #[error("cloudflare challenge did not resolve within {timeout_secs}s")]
    CloudflareTimeout { timeout_secs: u64 },

    #[error("element not found: {selector}")]
    ElementNotFound { selector: String },

    #[error("content extraction failed: {0}")]
    Extraction(String),

    #[error("screenshot failed: {0}")]
    Screenshot(#[source] anyhow::Error),

    #[error("browser task panicked")]
    TaskJoin(#[from] tokio::task::JoinError),

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}
