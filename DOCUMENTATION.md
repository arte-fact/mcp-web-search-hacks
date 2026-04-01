# Documentation

## Tools

### `fetch`

Fetch a URL and return its content as clean text. Uses a headless browser to handle JavaScript rendering and Cloudflare challenges.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `url` | string | yes | The URL to fetch |
| `timeout_secs` | number | no | Max seconds to wait for Cloudflare challenges (default: 10) |

### `search`

Search the web and return a list of results with titles, URLs, and snippets. Uses DuckDuckGo as the search backend.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `query` | string | yes | The search query |
| `max_results` | number | no | Maximum number of results to return (default: 10) |

### `screenshot`

Take a screenshot of a URL and return it as a base64-encoded PNG.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `url` | string | yes | The URL to screenshot |
| `timeout_secs` | number | no | Max seconds to wait for Cloudflare challenges (default: 10) |

### `interact`

Navigate to a URL and perform a sequence of browser actions. Returns the page content as text and a screenshot after all actions complete.

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `url` | string | yes | The URL to navigate to |
| `actions` | array | yes | Ordered list of actions to perform (see below) |
| `timeout_secs` | number | no | Max seconds to wait for Cloudflare challenges (default: 10) |

**Action object:**

| Field | Type | Description |
|-------|------|-------------|
| `action_type` | string | One of: `click`, `type_text`, `wait`, `scroll`, `press_key` |
| `selector` | string | CSS selector for the target element (for `click`/`type_text`) |
| `text` | string | Text to type (for `type_text`) |
| `key` | string | Key to press, e.g. `Enter`, `Tab` (for `press_key`, also sent after `type_text` if provided) |
| `delay_ms` | number | Milliseconds to wait (for `wait`, default: 1000) |

**Example -- search a site via its search box:**

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

---

## OAuth 2.1

The HTTP server implements OAuth 2.1 with mandatory PKCE (S256) for the MCP endpoint.

### Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /.well-known/oauth-protected-resource` | Protected Resource Metadata ([RFC 9728](https://www.rfc-editor.org/rfc/rfc9728)) |
| `GET /.well-known/oauth-authorization-server` | Authorization Server Metadata ([RFC 8414](https://www.rfc-editor.org/rfc/rfc8414)) |
| `POST /register` | Dynamic Client Registration ([RFC 7591](https://www.rfc-editor.org/rfc/rfc7591)) |
| `GET /authorize` | Authorization page (renders password form) |
| `POST /authorize` | Authorization submission |
| `POST /token` | Token exchange (PKCE required) |
| `POST /mcp` | MCP endpoint (requires `Authorization: Bearer <token>`) |

### Flow

1. **Register** -- Client sends `POST /register` with `client_name` and `redirect_uris`. Receives `client_id` and `client_secret`.
2. **Authorize** -- Client redirects user to `GET /authorize` with `response_type=code`, `client_id`, `redirect_uri`, `code_challenge` (S256), and `code_challenge_method=S256`. The server shows a password form.
3. **Approve** -- User enters the admin password. On success, the server redirects back with an authorization `code` (valid for 10 minutes).
4. **Token exchange** -- Client sends `POST /token` with `grant_type=authorization_code`, `code`, `client_id`, and `code_verifier`. Receives an `access_token` (valid for 1 hour).
5. **API calls** -- Client includes `Authorization: Bearer <access_token>` in requests to `POST /mcp`.

MCP clients like Claude Desktop and Claude Code handle this flow automatically.

---

## Admin Panel

The HTTP server includes a built-in admin panel at `/admin`. It uses the same admin password as OAuth authorization.

### Features

- **Dashboard** -- Total requests, requests/min, active clients, active tokens, uptime. Auto-refreshes every 5 seconds.
- **Request Logs** -- Paginated table of all MCP tool calls: timestamp, JSON-RPC method, tool name, parameters (truncated), duration, success/failure, and client token prefix. Stores the last 1000 entries in memory.
- **Client Management** -- List all registered OAuth clients with their redirect URIs, registration time, and active token count. Revoke a client (also revokes all its tokens).
- **Token Management** -- List all active access tokens with their client, creation time, and remaining TTL. Revoke individual tokens.

### Access

1. Navigate to `http://<server>/admin`
2. Enter the admin password
3. Session is cookie-based (`HttpOnly`, `SameSite=Strict`) and lasts 24 hours

### API Endpoints

All admin API endpoints are under `/admin/api/`. Except for login, they require an `admin_session` cookie.

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| `POST` | `/admin/api/login` | None | Authenticate with `{ "password": "..." }` |
| `POST` | `/admin/api/logout` | Cookie | Clear session |
| `GET` | `/admin/api/dashboard` | Cookie | Dashboard stats |
| `GET` | `/admin/api/logs?limit=N&offset=N` | Cookie | Paginated request logs (newest first) |
| `GET` | `/admin/api/clients` | Cookie | List OAuth clients |
| `DELETE` | `/admin/api/clients/:client_id` | Cookie | Revoke client and its tokens |
| `GET` | `/admin/api/tokens` | Cookie | List active tokens |
| `DELETE` | `/admin/api/tokens/:token_prefix` | Cookie | Revoke token by prefix |

### Data Retention

All admin data is stored in memory and resets when the server restarts. Request logs are capped at 1000 entries (oldest are dropped first).

---

## Logging

Server logs are written to **stderr** using the `tracing` crate. Control verbosity with the `RUST_LOG` environment variable:

```sh
RUST_LOG=info ./target/release/mcp-web-search-server --bind 0.0.0.0:3000 --base-url https://mcp.example.com
```

| Level | What it shows |
|-------|---------------|
| `error` | Failures only |
| `warn` | Warnings and errors |
| `info` | Startup, connections, token issuance |
| `debug` | Request details, browser operations |
| `trace` | Everything including rmcp protocol messages |

---

## Architecture

### Project Structure

```
mcp-web-search-hacks/
  crates/
    mcp-web-search-core/       Shared library
      src/
        server.rs              MCP tool definitions and ServerHandler
        browser.rs             Headless Chrome automation
        extraction.rs          HTML parsing and text extraction
        error.rs               Error types
    mcp-web-search-stdio/      Local stdio binary
      src/main.rs
    mcp-web-search-server/     Remote HTTP binary
      src/
        main.rs                Axum server setup and routing
        auth.rs                OAuth 2.1 implementation
        admin.rs               Admin panel API and logging middleware
      static/
        admin.html             Admin SPA (embedded at compile time)
  deploy/                      Traefik configuration
  Dockerfile                   Multi-stage build
  docker-compose.yml           Docker Compose with Traefik
```

### Transports

| Transport | Binary | Use case |
|-----------|--------|----------|
| **stdio** | `mcp-web-search-stdio` | Local use as subprocess (Claude Code, Claude Desktop) |
| **Streamable HTTP** | `mcp-web-search-server` | Remote/shared deployment with OAuth |

### Browser Management

A single headless Chrome instance is shared across all MCP sessions. Each tool call opens a new browser tab (via `TabGuard` RAII pattern that auto-closes on drop). Browser features:

- Anti-detection: custom User-Agent, `navigator.webdriver` override, plugin spoofing
- Cloudflare challenge detection and wait loop (checks page title and DOM selectors)
- Configurable timeout per request (default 10s)
- Text extraction via `html2text` crate (capped at 50,000 characters)
