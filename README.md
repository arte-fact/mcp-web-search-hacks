# mcp-web-search-hacks

An MCP (Model Context Protocol) server that gives LLMs web access through a headless Chrome browser. It handles JavaScript-rendered pages and Cloudflare challenges automatically.

## Features

- **fetch** — Retrieve any URL as clean, readable text
- **search** — Web search via DuckDuckGo with structured results
- **screenshot** — Capture full-page screenshots as PNG
- **interact** — Navigate to a page and perform actions (click, type, scroll, key press)

All tools use a real headless browser, so JS-heavy sites and Cloudflare-protected pages work out of the box.

## Prerequisites

- **Rust** (edition 2024)
- **Google Chrome** or **Chromium** installed and available on `PATH`

## Build

```sh
cargo build --release
```

The binary is at `target/release/mcp-web-search-hacks`.

## Usage

The server communicates over **stdio** using the MCP protocol. It is designed to be launched by an MCP client (e.g. Claude Code, Claude Desktop, or any MCP-compatible host).

### Claude Code

Add to your MCP config (`.claude/settings.json` or project settings):

```json
{
  "mcpServers": {
    "web-search": {
      "command": "/path/to/mcp-web-search-hacks"
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
      "command": "/path/to/mcp-web-search-hacks"
    }
  }
}
```

### Any MCP client

Launch the binary as a subprocess and communicate over stdin/stdout with the MCP protocol.

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
RUST_LOG=info ./target/release/mcp-web-search-hacks
RUST_LOG=debug ./target/release/mcp-web-search-hacks
```

## License

See [Cargo.toml](Cargo.toml) for package metadata.
