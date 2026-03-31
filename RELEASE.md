# mcp-web-search-hacks v0.1.0

Initial release of **mcp-web-search-hacks** — an MCP server that gives LLMs real web access through a headless Chrome browser, with automatic Cloudflare bypass.

## Features

### MCP Tools

- **fetch** — Retrieve any URL as clean, readable text. Handles JavaScript-rendered pages and Cloudflare challenges automatically.
- **search** — Search the web via DuckDuckGo and return structured results with titles, URLs, and snippets.
- **screenshot** — Capture full-page PNG screenshots of any URL.
- **interact** — Navigate to a page and perform actions: click elements, type text, press keys, scroll, and wait. Returns the final page content and a screenshot.

### Cloudflare Bypass

The server automatically detects and waits for Cloudflare "checking your browser" challenges to resolve. It uses a real Chromium engine, so JS-based challenges are solved natively. Configurable timeout (default: 10 seconds).

### Anti-Detection

- Spoofed user agent (Chrome 131 on Windows 10)
- `navigator.webdriver` hidden
- Fake browser plugins and language settings
- Chrome runtime object injection
- Automation-related Blink features disabled

### Content Extraction

- HTML-to-text conversion with clean formatting and link references
- DuckDuckGo result parsing with structured output
- Automatic text truncation at 50,000 characters

## Platform Support

Pre-built binaries are available for:

| Platform | Architecture | Format |
|----------|-------------|--------|
| Linux | x86_64 | tar.gz |
| Linux | aarch64 (ARM64) | tar.gz |
| macOS | x86_64 (Intel) | tar.gz |
| macOS | aarch64 (Apple Silicon) | tar.gz |
| Windows | x86_64 | zip |

## Prerequisites

- **Google Chrome** or **Chromium** must be installed and available on PATH.

## Configuration

### Claude Code

Add to your MCP settings (`.claude/settings.json` or project settings):

```json
{
  "mcpServers": {
    "mcp-web-search-hacks": {
      "command": "/path/to/mcp-web-search-hacks",
      "args": []
    }
  }
}
```

### Claude Desktop

Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "mcp-web-search-hacks": {
      "command": "/path/to/mcp-web-search-hacks",
      "args": []
    }
  }
}
```

## Building from Source

```bash
git clone <repo-url>
cd mcp-web-search-hacks
cargo build --release
```

The binary will be at `target/release/mcp-web-search-hacks`.
