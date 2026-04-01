# Installation

## Prerequisites

- **Rust** toolchain (edition 2024) -- install via [rustup](https://rustup.rs/)
- **Google Chrome** or **Chromium** installed and available on `PATH`

## Build from Source

```sh
git clone https://github.com/arte-fact/mcp-web-search-hacks.git
cd mcp-web-search-hacks
cargo build --release
```

Binaries are at:
- `target/release/mcp-web-search-stdio` -- local stdio server
- `target/release/mcp-web-search-server` -- remote HTTP server

## Client Setup

### Claude Code (stdio)

Add to your Claude Code MCP config (`.claude/settings.json` or project-level):

```json
{
  "mcpServers": {
    "web-search": {
      "command": "/path/to/mcp-web-search-stdio"
    }
  }
}
```

Claude Code launches the binary as a subprocess and communicates over stdin/stdout.

### Claude Code (remote HTTP)

If you have the HTTP server running (see [Remote Server](#remote-server) below):

```json
{
  "mcpServers": {
    "web-search": {
      "url": "https://mcp.example.com/mcp"
    }
  }
}
```

On first connection, Claude Code opens a browser window for OAuth authorization. Enter the admin password to approve access.

### Claude Desktop (stdio)

Add to `claude_desktop_config.json`:

| OS | Config path |
|----|-------------|
| macOS | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| Windows | `%APPDATA%\Claude\claude_desktop_config.json` |
| Linux | `~/.config/Claude/claude_desktop_config.json` |

```json
{
  "mcpServers": {
    "web-search": {
      "command": "/path/to/mcp-web-search-stdio"
    }
  }
}
```

Restart Claude Desktop after editing the config. The web-search tools will appear in the tools menu (hammer icon).

### Claude Desktop (remote HTTP)

```json
{
  "mcpServers": {
    "web-search": {
      "url": "https://mcp.example.com/mcp"
    }
  }
}
```

Claude Desktop handles the OAuth flow automatically -- a browser window opens for you to enter the admin password.

### llama.cpp Web UI

The llama.cpp web interface has a built-in MCP client that runs in the browser. It connects to MCP servers over HTTP (not stdio), so you need the **HTTP server binary**.

#### Step 1: Start the MCP server

```sh
MCP_ADMIN_PASSWORD=secret ./target/release/mcp-web-search-server \
  --bind 127.0.0.1:3001 \
  --base-url http://localhost:3001
```

#### Step 2: Start llama-server with MCP proxy enabled

The `--webui-mcp-proxy` flag is required. It adds a CORS proxy endpoint so the browser-based MCP client can reach your MCP server without cross-origin issues.

```sh
llama-server -m model.gguf --webui-mcp-proxy
```

Use a model that supports tool/function calling (e.g., Qwen, Llama 3.x, Mistral).

#### Step 3: Configure the MCP server in the Web UI

1. Open the llama.cpp web UI (default: `http://localhost:8080`)
2. Go to **Settings** (gear icon) > **MCP Client**
3. Click **Add Server** and enter:
   - **URL**: `http://localhost:3001/mcp`
   - **Name**: `web-search` (optional)
4. Click the **edit icon** on the entry and toggle **"use llama-server proxy"** on
5. Enable the server entry

The web UI will connect to the MCP server through the CORS proxy and discover the available tools (`fetch`, `search`, `screenshot`, `interact`).

#### Pre-configure via command line (optional)

You can skip the UI setup by passing MCP config at startup:

```sh
llama-server -m model.gguf --webui-mcp-proxy \
  --webui-config '{"mcpServers": "[{\"enabled\":true,\"url\":\"http://localhost:3001/mcp\",\"name\":\"web-search\",\"useProxy\":true}]"}'
```

Or via a config file:

```sh
llama-server -m model.gguf --webui-mcp-proxy --webui-config-file webui-config.json
```

`webui-config.json`:
```json
{
  "mcpServers": "[{\"enabled\":true,\"url\":\"http://localhost:3001/mcp\",\"name\":\"web-search\",\"useProxy\":true}]"
}
```

> **Note:** The `mcpServers` value is a JSON string containing a JSON array (this is how the web UI stores it in localStorage).

#### Limitations

- **No stdio support** -- the llama.cpp MCP client runs in the browser, so it can only connect to HTTP-based MCP servers.
- **OAuth not supported** -- the llama.cpp web UI does not implement the OAuth 2.1 flow. For local use, you can either disable auth or set up a reverse proxy that injects a Bearer token.
- **Transport fallback** -- Streamable HTTP is tried first; if it fails, legacy SSE is attempted automatically.

## Remote Server

The HTTP server exposes an MCP endpoint at `/mcp` with OAuth 2.1 authentication (authorization code flow with mandatory PKCE).

### Run locally

```sh
MCP_ADMIN_PASSWORD=secret cargo run -p mcp-web-search-server -- \
  --bind 127.0.0.1:3000 \
  --base-url http://localhost:3000
```

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
| `MCP_ADMIN_PASSWORD` | **Required.** Password for OAuth authorization and admin panel |
| `MCP_BASE_URL` | Public URL of the server (used in OAuth metadata) |
| `MCP_BIND` | Bind address (default: `127.0.0.1:3000`) |
| `RUST_LOG` | Log level: `error`, `warn`, `info`, `debug`, `trace` |
