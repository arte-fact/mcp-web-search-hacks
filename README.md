# mcp-web-search-hacks

An MCP (Model Context Protocol) server that gives LLMs web access through a headless Chrome browser. It handles JavaScript-rendered pages and Cloudflare challenges automatically.

## Features

- **fetch** -- Retrieve any URL as clean, readable text
- **search** -- Web search via DuckDuckGo with structured results
- **screenshot** -- Capture full-page screenshots as PNG
- **interact** -- Navigate to a page and perform actions (click, type, scroll, key press)
- **Admin panel** -- Built-in web UI to monitor requests, manage OAuth clients, and revoke tokens

All tools use a real headless browser, so JS-heavy sites and Cloudflare-protected pages work out of the box.

## Quick Start

```sh
cargo build --release
```

**Local (stdio)** -- configure in your MCP client:
```json
{
  "mcpServers": {
    "web-search": { "command": "/path/to/mcp-web-search-stdio" }
  }
}
```

**Remote (HTTP)** -- start the server, then connect:
```sh
MCP_ADMIN_PASSWORD=secret ./target/release/mcp-web-search-server \
  --bind 127.0.0.1:3000 --base-url http://localhost:3000
```
```json
{
  "mcpServers": {
    "web-search": { "url": "https://mcp.example.com/mcp" }
  }
}
```

Admin panel is available at `http://<server>/admin`.

## Project Structure

| Crate | Description |
|-------|-------------|
| `mcp-web-search-core` | Shared library: browser automation, extraction, server handler |
| `mcp-web-search-stdio` | Binary: local MCP server over stdio |
| `mcp-web-search-server` | Binary: remote HTTP server with OAuth 2.1 and admin panel |

## Documentation

- **[INSTALL.md](INSTALL.md)** -- Prerequisites, build instructions, client setup (Claude Code, Claude Desktop, llama.cpp), remote server deployment
- **[DOCUMENTATION.md](DOCUMENTATION.md)** -- Tool reference, OAuth flow, admin panel API, architecture overview, logging
