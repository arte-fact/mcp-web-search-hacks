# mcp-web-search-hacks

An MCP (Model Context Protocol) server that gives LLMs web access through a headless Chrome browser. It handles JavaScript-rendered pages and Cloudflare challenges automatically.

## Features

- **fetch** — Retrieve any URL as clean, readable text
- **search** — Web search via DuckDuckGo with structured results
- **screenshot** — Capture full-page screenshots as PNG
- **interact** — Navigate to a page and perform actions (click, type, scroll, key press)

All tools use a real headless browser, so JS-heavy sites and Cloudflare-protected pages work out of the box.

## Project Structure

This is a Cargo workspace with three crates:

| Crate | Description |
|-------|-------------|
| `mcp-web-search-core` | Shared library: browser automation, extraction, server handler |
| `mcp-web-search-stdio` | Binary: local MCP server over stdio |
| `mcp-web-search-server` | Binary: remote HTTP server with OAuth 2.1 |

## Prerequisites

- **Rust** (edition 2024)
- **Google Chrome** or **Chromium** installed and available on `PATH`

## Build

```sh
cargo build --release
```

Binaries are at:
- `target/release/mcp-web-search-stdio` — local stdio server
- `target/release/mcp-web-search-server` — remote HTTP server

## Local Usage (stdio)

The stdio binary communicates over stdin/stdout using the MCP protocol. It is designed to be launched by an MCP client as a subprocess.

### Claude Code

```json
{
  "mcpServers": {
    "web-search": {
      "command": "/path/to/mcp-web-search-stdio"
    }
  }
}
```

### Claude Desktop

Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "web-search": {
      "command": "/path/to/mcp-web-search-stdio"
    }
  }
}
```

## Remote Usage (HTTP + OAuth)

The HTTP server exposes an MCP endpoint at `/mcp` with OAuth 2.1 authentication (authorization code flow with PKCE). MCP clients like Claude Desktop and Claude Code handle the OAuth flow automatically.

### Run locally

```sh
MCP_ADMIN_PASSWORD=secret cargo run -p mcp-web-search-server -- \
  --bind 127.0.0.1:3000 \
  --base-url http://localhost:3000
```

### Connect from Claude Code

```json
{
  "mcpServers": {
    "web-search": {
      "url": "https://mcp.example.com/mcp"
    }
  }
}
```

When connecting for the first time, the client will open a browser window for you to enter the admin password.

### Deploy with Docker Compose + Traefik

1. Copy `.env.example` to `.env` and fill in your values:

```sh
cp .env.example .env
```

```
MCP_HOST=mcp.example.com
MCP_BASE_URL=https://mcp.example.com
MCP_ADMIN_PASSWORD=your-secure-password
ACME_EMAIL=you@example.com
```

2. Start the stack:

```sh
docker compose up -d
```

Traefik handles HTTPS with automatic Let's Encrypt certificates.

### Environment Variables

| Variable | Description |
|----------|-------------|
| `MCP_ADMIN_PASSWORD` | **Required.** Password for OAuth authorization |
| `MCP_BASE_URL` | Public URL of the server (used in OAuth metadata) |
| `MCP_BIND` | Bind address (default: `127.0.0.1:3000`) |
| `RUST_LOG` | Log level: `error`, `warn`, `info`, `debug`, `trace` |

### OAuth Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /.well-known/oauth-protected-resource` | Protected Resource Metadata (RFC 9728) |
| `GET /.well-known/oauth-authorization-server` | Authorization Server Metadata (RFC 8414) |
| `POST /register` | Dynamic Client Registration (RFC 7591) |
| `GET /authorize` | Authorization page |
| `POST /token` | Token exchange |
| `POST /mcp` | MCP endpoint (requires Bearer token) |

## Tools

### `fetch`

Fetch a URL and return its content as clean text.

| Parameter      | Type   | Required | Description                                          |
|----------------|--------|----------|------------------------------------------------------|
| `url`          | string | yes      | The URL to fetch                                     |
| `timeout_secs` | number | no       | Max seconds to wait for Cloudflare challenges (default: 10) |

### `search`

Search the web and return a list of results with titles, URLs, and snippets.

| Parameter     | Type   | Required | Description                                    |
|---------------|--------|----------|------------------------------------------------|
| `query`       | string | yes      | The search query                               |
| `max_results` | number | no       | Maximum number of results to return (default: 10) |

### `screenshot`

Take a screenshot of a URL and return it as a base64-encoded PNG.

| Parameter      | Type   | Required | Description                                          |
|----------------|--------|----------|------------------------------------------------------|
| `url`          | string | yes      | The URL to screenshot                                |
| `timeout_secs` | number | no       | Max seconds to wait for Cloudflare challenges (default: 10) |

### `interact`

Navigate to a URL and perform a sequence of browser actions. Returns the page content as text and a screenshot after all actions complete.

| Parameter      | Type     | Required | Description                                          |
|----------------|----------|----------|------------------------------------------------------|
| `url`          | string   | yes      | The URL to navigate to                               |
| `actions`      | array    | yes      | Ordered list of actions to perform (see below)       |
| `timeout_secs` | number   | no       | Max seconds to wait for Cloudflare challenges (default: 10) |

**Action object:**

| Field         | Type   | Description                                                |
|---------------|--------|------------------------------------------------------------|
| `action_type` | string | One of: `click`, `type_text`, `wait`, `scroll`, `press_key` |
| `selector`    | string | CSS selector for the target element (for `click`/`type_text`) |
| `text`        | string | Text to type (for `type_text`)                             |
| `key`         | string | Key to press, e.g. `Enter`, `Tab` (for `press_key`, also sent after `type_text` if provided) |
| `delay_ms`    | number | Milliseconds to wait (for `wait`, default: 1000)           |

**Example — search a site via its search box:**

```json
{
  "url": "https://example.com",
  "actions": [
    { "action_type": "click", "selector": "input[name='q']" },
    { "action_type": "type_text", "selector": "input[name='q']", "text": "hello", "key": "Enter" },
    { "action_type": "wait", "delay_ms": 2000 }
  ]
}
```

## Logging

Logs are written to **stderr** using `tracing`. Control verbosity with the `RUST_LOG` environment variable:

```sh
RUST_LOG=info ./target/release/mcp-web-search-server --bind 0.0.0.0:3000 --base-url https://mcp.example.com
```
