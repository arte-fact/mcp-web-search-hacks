# Code Quality Audit: mcp-web-search-hacks

This document outlines the results of a code quality and best practices audit performed on the `mcp-web-search-hacks` project, specifically focusing on the browser automation core.

## 🛠️ Architecture Analysis

The project is well-structured for a headless browser service. Key strengths include:

- **Resilience**: Implementation of a `generation` counter and `lock_unpoisoned` pattern allows the server to recover from browser crashes without requiring a full process restart.
- **Resource Management**: The `TabGuard` (RAII) pattern ensures that browser tabs are closed reliably, preventing memory leaks in the headless Chrome process.
- **Safety**: Budget enforcement via `with_budget` wrappers prevents long-running or hung requests from exhausting server worker threads.
- **Anti-Detection**: Implementation of basic bot-detection bypasses (e.g., modifying `navigator.webdriver`).

## 🚩 Areas for Improvement

### 1. Error Contextualization
Currently, many errors are mapped to a generic `Navigation` error with empty context. 
- **Issue**: Losing the specific URL or action (click, type, etc.) that caused the failure makes debugging production logs difficult.
- **Recommendation**: Introduce a `BrowserOperationError` enum that captures the operation type, target element, and the underlying `headless_chrome` error.

### 2. Magic Numbers
The codebase contains several hardcoded durations (e.g., `300ms`, `500ms` sleeps) used for stability.
- **Issue**: These are "magic numbers" that are difficult to tune across different environments or targets.
- **Recommendation**: Extract these into a `BrowserConfig` struct or named constants (e.g., `DEFAULT_INTERACTION_DELAY`).

### 3. User Agent Management
The User Agent is currently hardcoded.
- **Issue**: As Chrome updates, a static UA becomes a fingerprint that can be used to identify the bot.
- **Recommendation**: Implement a configurable User Agent via environment variables (`MCP_USER_AGENT`) or integrate a library that rotates common UAs.

### 4. Observability
While `tracing` is used, some critical paths (like the `spawn_blocking` interaction loops) lack detailed instrumentation.
- **Issue**: It is difficult to tell exactly which step of a complex interaction failed without adding print statements.
- **Recommendation**: Add `tracing::info!` and `tracing::debug!` calls before and after each browser interaction.

## 🚀 Summary of Proposed Changes

| Category | Proposed Change | Impact |
| :--- | :--- | :--- |
| **Errors** | Refactor to `BrowserOperationError` | High (Debuggability) |
| **Config** | Move magic numbers to `BrowserConfig` | Medium (Maintainability) |
| **Detection** | Dynamic/Configurable User Agent | High (Reliability) |
| **Logs** | Enhanced `tracing` instrumentation | Medium (Observability) |
