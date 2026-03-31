mod auth;

use std::sync::Arc;

use axum::Router;
use clap::Parser;
use mcp_web_search_core::{browser::BrowserManager, server::WebServer};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "mcp-web-search-server",
    about = "Remotely hostable MCP web search server"
)]
struct Args {
    /// Bind address
    #[arg(long, default_value = "127.0.0.1:3000", env = "MCP_BIND")]
    bind: String,

    /// Public base URL (used in OAuth metadata)
    #[arg(long, env = "MCP_BASE_URL")]
    base_url: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();
    let admin_password =
        std::env::var("MCP_ADMIN_PASSWORD").expect("MCP_ADMIN_PASSWORD env var must be set");

    tracing::info!("launching headless browser...");
    let browser = Arc::new(
        BrowserManager::launch().map_err(|e| anyhow::anyhow!("failed to launch browser: {e}"))?,
    );
    tracing::info!("browser launched successfully");

    let ct = CancellationToken::new();

    // MCP service — creates a fresh WebServer per session, sharing one browser
    let browser_for_factory = browser.clone();
    let mcp_service = StreamableHttpService::new(
        move || Ok(WebServer::new_with_arc(browser_for_factory.clone())),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default().with_cancellation_token(ct.child_token()),
    );

    let oauth_state = Arc::new(auth::OAuthState::new(admin_password, args.base_url.clone()));

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Public OAuth endpoints
    let oauth_routes = auth::router(oauth_state.clone()).layer(cors);

    // Protected MCP endpoint
    let mcp_routes = Router::new().nest_service("/mcp", mcp_service).layer(
        axum::middleware::from_fn_with_state(oauth_state.clone(), auth::auth_middleware),
    );

    let app = Router::new().merge(oauth_routes).merge(mcp_routes);

    let listener = TcpListener::bind(&args.bind).await?;
    tracing::info!("MCP HTTP server listening on {}", args.bind);
    tracing::info!("base URL: {}", args.base_url);

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("shutting down...");
            ct.cancel();
        })
        .await?;

    Ok(())
}
