mod browser;
mod error;
mod extraction;
mod server;

use rmcp::ServiceExt;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    tracing::info!("launching headless browser...");
    let browser_manager = browser::BrowserManager::launch()
        .map_err(|e| anyhow::anyhow!("failed to launch browser: {e}"))?;
    tracing::info!("browser launched successfully");

    let server = server::WebServer::new(browser_manager);

    tracing::info!("starting MCP server on stdio...");
    let service = server.serve(rmcp::transport::io::stdio()).await?;
    let quit_reason = service.waiting().await?;

    tracing::info!(?quit_reason, "server shut down");
    Ok(())
}
