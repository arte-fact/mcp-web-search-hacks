# CLAUDE.md

## Build & Check
- `cargo build --workspace` - build all crates
- `cargo clippy --workspace` - lint (CI runs with `-D warnings`)
- `cargo fmt --all -- --check` - format check
- No test suite exists; verification is manual (start server, call tools)

## Project Layout
- Cargo workspace with 3 crates under `crates/`
- `mcp-web-search-core` - shared library (tools, browser, extraction)
- `mcp-web-search-stdio` - stdio binary (local MCP)
- `mcp-web-search-server` - HTTP binary (axum, OAuth, admin panel)
- Admin SPA is a single HTML file at `crates/mcp-web-search-server/static/admin.html`, embedded via `include_str!`

## Key Patterns
- MCP tools defined with rmcp `#[tool]` / `#[tool_router]` / `#[tool_handler]` macros in `server.rs`
- Tool params use `schemars::JsonSchema` for auto schema generation
- Tool errors return `Ok(CallToolResult::error(...))`, not `Err(McpError)` (McpError is for protocol errors)
- Auth state uses `tokio::sync::RwLock<HashMap<>>` for concurrent access
- Admin state is separate from OAuth state; handlers take `Arc<(Arc<OAuthState>, Arc<AdminState>)>`
- Axum layers apply bottom-to-top: last `.layer()` runs first

## Environment
- `MCP_ADMIN_PASSWORD` (required for HTTP server)
- `MCP_BASE_URL`, `MCP_BIND`, `RUST_LOG`
- Needs Chrome/Chromium on PATH

## Docs
- README.md is concise overview; details are in INSTALL.md and DOCUMENTATION.md
