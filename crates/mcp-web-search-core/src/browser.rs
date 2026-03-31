use std::ffi::OsStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use headless_chrome::{Browser, LaunchOptions, Tab};

use crate::error::Error;

pub struct BrowserManager {
    browser: Arc<Browser>,
    default_timeout: Duration,
    user_agent: String,
}

impl std::fmt::Debug for BrowserManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserManager")
            .field("default_timeout", &self.default_timeout)
            .field("user_agent", &self.user_agent)
            .finish_non_exhaustive()
    }
}

struct TabGuard(Arc<Tab>);

impl Drop for TabGuard {
    fn drop(&mut self) {
        self.0.close(true).ok();
    }
}

impl BrowserManager {
    pub fn launch() -> Result<Self, Error> {
        let launch_options = LaunchOptions::default_builder()
            .headless(true)
            .sandbox(false)
            .window_size(Some((1920, 1080)))
            .idle_browser_timeout(Duration::from_secs(600))
            .args(vec![
                OsStr::new("--disable-blink-features=AutomationControlled"),
                OsStr::new("--disable-features=IsolateOrigins,site-per-process"),
                OsStr::new("--disable-infobars"),
                OsStr::new("--no-first-run"),
            ])
            .build()
            .map_err(|e| Error::BrowserLaunch(e.into()))?;

        let browser = Browser::new(launch_options).map_err(Error::BrowserLaunch)?;

        Ok(Self {
            browser: Arc::new(browser),
            default_timeout: Duration::from_secs(10),
            user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                         (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
                .into(),
        })
    }

    pub async fn fetch_page(
        &self,
        url: String,
        timeout_secs: Option<u64>,
    ) -> Result<String, Error> {
        let browser = self.browser.clone();
        let ua = self.user_agent.clone();
        let timeout = self.resolve_timeout(timeout_secs);

        tokio::task::spawn_blocking(move || {
            let tab = setup_tab(&browser, &ua)?;
            let _guard = TabGuard(tab.clone());
            navigate_and_wait_for_cf(&tab, &url, timeout)?;
            tab.get_content()
                .map_err(|e| Error::Extraction(e.to_string()))
        })
        .await?
    }

    pub async fn screenshot_page(
        &self,
        url: String,
        timeout_secs: Option<u64>,
    ) -> Result<Vec<u8>, Error> {
        let browser = self.browser.clone();
        let ua = self.user_agent.clone();
        let timeout = self.resolve_timeout(timeout_secs);

        tokio::task::spawn_blocking(move || {
            let tab = setup_tab(&browser, &ua)?;
            let _guard = TabGuard(tab.clone());
            navigate_and_wait_for_cf(&tab, &url, timeout)?;
            tab.capture_screenshot(
                headless_chrome::protocol::cdp::Page::CaptureScreenshotFormatOption::Png,
                None,
                None,
                true,
            )
            .map_err(Error::Screenshot)
        })
        .await?
    }

    pub async fn search(&self, query: String, timeout_secs: Option<u64>) -> Result<String, Error> {
        let url = format!(
            "https://html.duckduckgo.com/html/?q={}",
            urlencoding::encode(&query)
        );
        self.fetch_page(url, timeout_secs).await
    }

    pub async fn interact_page(
        &self,
        url: String,
        actions: Vec<PageAction>,
        timeout_secs: Option<u64>,
    ) -> Result<InteractResult, Error> {
        let browser = self.browser.clone();
        let ua = self.user_agent.clone();
        let timeout = self.resolve_timeout(timeout_secs);

        tokio::task::spawn_blocking(move || {
            let tab = setup_tab(&browser, &ua)?;
            let _guard = TabGuard(tab.clone());
            navigate_and_wait_for_cf(&tab, &url, timeout)?;

            for action in &actions {
                execute_action(&tab, action)?;
            }

            // Wait for any final JS rendering
            std::thread::sleep(Duration::from_millis(500));

            let html = tab
                .get_content()
                .map_err(|e| Error::Extraction(e.to_string()))?;

            let final_url = tab.get_url();

            let screenshot = tab
                .capture_screenshot(
                    headless_chrome::protocol::cdp::Page::CaptureScreenshotFormatOption::Png,
                    None,
                    None,
                    true,
                )
                .ok()
                .map(|bytes| {
                    use base64::Engine;
                    base64::engine::general_purpose::STANDARD.encode(&bytes)
                });

            Ok(InteractResult {
                html,
                final_url,
                screenshot_b64: screenshot,
            })
        })
        .await?
    }

    fn resolve_timeout(&self, timeout_secs: Option<u64>) -> Duration {
        Duration::from_secs(timeout_secs.unwrap_or(self.default_timeout.as_secs()))
    }
}

pub struct InteractResult {
    pub html: String,
    pub final_url: String,
    pub screenshot_b64: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct PageAction {
    #[schemars(description = "Action type: click, type_text, wait, scroll, press_key")]
    pub action_type: ActionType,
    #[schemars(description = "CSS selector for the target element (for click/type_text)")]
    pub selector: Option<String>,
    #[schemars(description = "Text to type (for type_text action)")]
    pub text: Option<String>,
    #[schemars(description = "Key to press, e.g. 'Enter', 'Tab' (for press_key action)")]
    pub key: Option<String>,
    #[schemars(description = "Milliseconds to wait (for wait action)")]
    pub delay_ms: Option<u64>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    Click,
    TypeText,
    Wait,
    Scroll,
    PressKey,
}

fn setup_tab(browser: &Browser, user_agent: &str) -> Result<Arc<Tab>, Error> {
    let tab = browser.new_tab().map_err(Error::BrowserLaunch)?;

    tab.set_user_agent(user_agent, Some("en-US,en;q=0.9"), Some("Win32"))
        .map_err(Error::BrowserLaunch)?;

    // Inject anti-detection JS
    tab.evaluate(
        r#"
        Object.defineProperty(navigator, 'webdriver', { get: () => undefined });
        Object.defineProperty(navigator, 'plugins', { get: () => [1, 2, 3, 4, 5] });
        Object.defineProperty(navigator, 'languages', { get: () => ['en-US', 'en'] });
        window.chrome = { runtime: {} };
        "#,
        false,
    )
    .ok(); // best-effort before any page loads

    Ok(tab)
}

fn navigate_and_wait_for_cf(tab: &Tab, url: &str, timeout: Duration) -> Result<(), Error> {
    tab.navigate_to(url).map_err(|e| Error::Navigation {
        url: url.to_string(),
        source: e,
    })?;
    tab.wait_until_navigated().map_err(|e| Error::Navigation {
        url: url.to_string(),
        source: e,
    })?;

    let start = Instant::now();
    loop {
        if !is_cloudflare_challenge(tab) {
            break;
        }
        if start.elapsed() > timeout {
            return Err(Error::CloudflareTimeout {
                timeout_secs: timeout.as_secs(),
            });
        }
        std::thread::sleep(Duration::from_millis(500));
    }

    // Extra wait for JS rendering after challenge clears
    std::thread::sleep(Duration::from_millis(500));
    Ok(())
}

fn is_cloudflare_challenge(tab: &Tab) -> bool {
    if let Ok(title) = tab.get_title() {
        let t = title.to_lowercase();
        if t.contains("just a moment") || t.contains("attention required") {
            return true;
        }
    }

    if let Ok(result) = tab.evaluate(
        r#"
        !!(document.getElementById('cf-browser-verification')
           || document.getElementById('challenge-running')
           || document.querySelector('.cf-browser-verification'))
        "#,
        false,
    ) && let Some(serde_json::Value::Bool(true)) = result.value
    {
        return true;
    }

    false
}

fn execute_action(tab: &Tab, action: &PageAction) -> Result<(), Error> {
    match action.action_type {
        ActionType::Click => {
            let selector = action
                .selector
                .as_deref()
                .ok_or_else(|| Error::ElementNotFound {
                    selector: "(no selector provided)".into(),
                })?;
            tab.wait_for_element(selector)
                .map_err(|_| Error::ElementNotFound {
                    selector: selector.to_string(),
                })?
                .click()
                .map_err(|e| Error::Navigation {
                    url: String::new(),
                    source: e,
                })?;
            std::thread::sleep(Duration::from_millis(300));
        }
        ActionType::TypeText => {
            let selector = action
                .selector
                .as_deref()
                .ok_or_else(|| Error::ElementNotFound {
                    selector: "(no selector provided)".into(),
                })?;
            let text = action.text.as_deref().unwrap_or_default();
            let element = tab
                .wait_for_element(selector)
                .map_err(|_| Error::ElementNotFound {
                    selector: selector.to_string(),
                })?;
            element.click().map_err(|e| Error::Navigation {
                url: String::new(),
                source: e,
            })?;
            element.type_into(text).map_err(|e| Error::Navigation {
                url: String::new(),
                source: e,
            })?;
            if let Some(key) = &action.key {
                tab.press_key(key).map_err(|e| Error::Navigation {
                    url: String::new(),
                    source: e,
                })?;
                std::thread::sleep(Duration::from_millis(500));
            }
        }
        ActionType::Wait => {
            let delay = action.delay_ms.unwrap_or(1000);
            std::thread::sleep(Duration::from_millis(delay));
        }
        ActionType::Scroll => {
            tab.evaluate("window.scrollBy(0, window.innerHeight)", false)
                .map_err(|e| Error::Navigation {
                    url: String::new(),
                    source: e,
                })?;
            std::thread::sleep(Duration::from_millis(300));
        }
        ActionType::PressKey => {
            let key = action.key.as_deref().unwrap_or("Enter");
            tab.press_key(key).map_err(|e| Error::Navigation {
                url: String::new(),
                source: e,
            })?;
            std::thread::sleep(Duration::from_millis(300));
        }
    }
    Ok(())
}
