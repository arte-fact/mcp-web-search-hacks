use std::sync::Arc;

use rmcp::model::{CallToolResult, Content, ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, tool};

use crate::browser::BrowserManager;

#[derive(Debug, Clone)]
pub struct WebServer {
    browser: Arc<BrowserManager>,
}

impl WebServer {
    pub fn new(browser: BrowserManager) -> Self {
        Self {
            browser: Arc::new(browser),
        }
    }
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FetchParams {
    #[schemars(description = "The URL to fetch")]
    pub url: String,
    #[schemars(description = "Max seconds to wait for Cloudflare challenges (default: 10)")]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    #[schemars(description = "The search query")]
    pub query: String,
    #[schemars(description = "Maximum number of results to return (default: 10)")]
    pub max_results: Option<usize>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ScreenshotParams {
    #[schemars(description = "The URL to screenshot")]
    pub url: String,
    #[schemars(description = "Max seconds to wait for Cloudflare challenges (default: 10)")]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct InteractParams {
    #[schemars(description = "The URL to navigate to")]
    pub url: String,
    #[schemars(description = "Ordered list of actions to perform on the page")]
    pub actions: Vec<crate::browser::PageAction>,
    #[schemars(description = "Max seconds to wait for Cloudflare challenges (default: 10)")]
    pub timeout_secs: Option<u64>,
}

const MAX_TEXT_LENGTH: usize = 50_000;

fn truncate_text(text: String) -> String {
    if text.len() > MAX_TEXT_LENGTH {
        format!(
            "{}...\n[truncated, {} total chars]",
            &text[..MAX_TEXT_LENGTH],
            text.len()
        )
    } else {
        text
    }
}

#[tool(tool_box)]
impl WebServer {
    #[tool(
        description = "Fetch a URL and return its content as clean text. Uses a headless browser to handle JavaScript rendering and Cloudflare challenges."
    )]
    async fn fetch(
        &self,
        #[tool(aggr)] params: FetchParams,
    ) -> Result<CallToolResult, rmcp::Error> {
        match self
            .browser
            .fetch_page(params.url, params.timeout_secs)
            .await
        {
            Ok(html) => {
                let text = crate::extraction::html_to_text(&html, 80);
                Ok(CallToolResult::success(vec![Content::text(truncate_text(
                    text,
                ))]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    #[tool(
        description = "Search the web and return a list of results with titles, URLs, and snippets."
    )]
    async fn search(
        &self,
        #[tool(aggr)] params: SearchParams,
    ) -> Result<CallToolResult, rmcp::Error> {
        match self.browser.search(params.query, None).await {
            Ok(html) => {
                let mut results = crate::extraction::parse_duckduckgo_results(&html);
                let max = params.max_results.unwrap_or(10);
                results.truncate(max);
                if results.is_empty() {
                    return Ok(CallToolResult::success(vec![Content::text(
                        "No results found.",
                    )]));
                }
                let formatted = results
                    .iter()
                    .enumerate()
                    .map(|(i, r)| {
                        format!(
                            "{}. {}\n   URL: {}\n   {}",
                            i + 1,
                            r.title,
                            r.url,
                            r.snippet
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                Ok(CallToolResult::success(vec![Content::text(formatted)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    #[tool(description = "Take a screenshot of a URL and return it as a base64-encoded PNG image.")]
    async fn screenshot(
        &self,
        #[tool(aggr)] params: ScreenshotParams,
    ) -> Result<CallToolResult, rmcp::Error> {
        match self
            .browser
            .screenshot_page(params.url, params.timeout_secs)
            .await
        {
            Ok(png_bytes) => {
                use base64::Engine;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
                Ok(CallToolResult::success(vec![Content::image(
                    b64,
                    "image/png",
                )]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }

    #[tool(
        description = "Navigate to a URL and perform a sequence of interactions (click, type_text, wait, scroll, press_key). Returns the page content and a screenshot after all actions complete."
    )]
    async fn interact(
        &self,
        #[tool(aggr)] params: InteractParams,
    ) -> Result<CallToolResult, rmcp::Error> {
        match self
            .browser
            .interact_page(params.url, params.actions, params.timeout_secs)
            .await
        {
            Ok(result) => {
                let text = crate::extraction::html_to_text(&result.html, 80);
                let mut parts = vec![Content::text(truncate_text(format!(
                    "Final URL: {}\n\n{}",
                    result.final_url, text
                )))];
                if let Some(screenshot_b64) = result.screenshot_b64 {
                    parts.push(Content::image(screenshot_b64, "image/png"));
                }
                Ok(CallToolResult::success(parts))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e.to_string())])),
        }
    }
}

#[tool(tool_box)]
impl ServerHandler for WebServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some(
                "Web access server with Cloudflare bypass. Provides tools to fetch web pages, \
                 search the web, take screenshots, and interact with pages. Handles JavaScript \
                 rendering and Cloudflare challenges automatically using a headless browser."
                    .into(),
            ),
            ..Default::default()
        }
    }
}
