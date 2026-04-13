mod admin;
mod auth;

use std::sync::Arc;
use std::time::Duration;

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

    // MCP service — creates a fresh WebServer per session, sharing one browser.
    // SSE keep-alive at 15s: default in rmcp 1.3, but set explicitly so intent is
    // visible. Keeps idle streams alive through intermediate proxies.
    let browser_for_factory = browser.clone();
    let mcp_service = StreamableHttpService::new(
        move || Ok(WebServer::new_with_arc(browser_for_factory.clone())),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default()
            .with_cancellation_token(ct.child_token())
            .with_sse_keep_alive(Some(Duration::from_secs(15))),
    );

    let oauth_state = Arc::new(auth::OAuthState::new(admin_password, args.base_url.clone()));
    let admin_state = Arc::new(admin::AdminState::new());

    // Background eviction: prune expired auth codes, access tokens, refresh
    // tokens, and admin sessions so state can't grow unboundedly.
    spawn_eviction_task(oauth_state.clone(), admin_state.clone(), ct.child_token());

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Public OAuth endpoints
    let oauth_routes = auth::router(oauth_state.clone()).layer(cors);

    // Protected MCP endpoint with request logging
    let mcp_routes = Router::new()
        .nest_service("/mcp", mcp_service)
        .layer(axum::middleware::from_fn_with_state(
            admin_state.clone(),
            admin::logging_middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            oauth_state.clone(),
            auth::auth_middleware,
        ));

    // Admin panel
    let admin_routes = admin::router(oauth_state.clone(), admin_state.clone());

    let app = Router::new()
        .merge(oauth_routes)
        .merge(mcp_routes)
        .merge(admin_routes);

    let listener = TcpListener::bind(&args.bind).await?;
    tracing::info!("MCP HTTP server listening on {}", args.bind);
    tracing::info!("base URL: {}", args.base_url);
    tracing::info!("admin panel: {}/admin", args.base_url);

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("shutting down...");
            ct.cancel();
        })
        .await?;

    Ok(())
}

fn spawn_eviction_task(
    oauth: Arc<auth::OAuthState>,
    admin: Arc<admin::AdminState>,
    ct: CancellationToken,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let (codes, access, refresh) = oauth.evict_expired().await;
                    let sessions = admin.evict_expired_sessions().await;
                    if codes + access + refresh + sessions > 0 {
                        tracing::info!(
                            codes, access, refresh, admin_sessions = sessions,
                            "evicted expired auth state",
                        );
                    }
                }
                _ = ct.cancelled() => break,
            }
        }
        tracing::info!("eviction task stopped");
    });
}
