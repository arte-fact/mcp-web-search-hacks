use std::ffi::OsStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use headless_chrome::{Browser, LaunchOptions, Tab};

use crate::error::Error;

pub struct BrowserManager {
    state: Arc<Mutex<BrowserState>>,
    default_timeout: Duration,
    user_agent: String,
}

struct BrowserState {
    browser: Arc<Browser>,
    generation: u64,
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
        if let Err(e) = self.0.close(true) {
            tracing::warn!(error = %e, "tab close failed; may leak browser resources");
        }
    }
}

/// Accept a poisoned lock guard instead of panicking. The data under the lock
/// (a shared Arc<Browser> and a u64 generation counter) is not structurally
/// invalid just because a prior holder panicked — clearing the poison lets the
/// next request observe the failure and relaunch Chrome rather than permanently
/// wedging every future request.
fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn launch_browser() -> Result<Browser, Error> {
    let launch_options = LaunchOptions::default_builder()
        .headless(true)
        .sandbox(false)
        .window_size(Some((1920, 1080)))
        .args(vec![
            OsStr::new("--disable-blink-features=AutomationControlled"),
            OsStr::new("--disable-features=IsolateOrigins,site-per-process"),
            OsStr::new("--disable-infobars"),
            OsStr::new("--no-first-run"),
        ])
        .build()
        .map_err(|e| Error::BrowserLaunch(e.into()))?;

    Browser::new(launch_options).map_err(Error::BrowserLaunch)
}

/// Mark the current Chrome instance dead so the next request relaunches it.
/// Called after a hard timeout, which indicates the browser may be wedged even
/// if it hasn't returned an error to us.
fn mark_browser_dead(state: &Mutex<BrowserState>) {
    let mut s = lock_unpoisoned(state);
    s.generation = s.generation.wrapping_add(1);
    // Drop the current Arc<Browser> reference — a fresh launch happens lazily
    // when the next `new_tab_or_relaunch` call sees `setup_tab` fail.
    tracing::warn!(
        generation = s.generation,
        "marked browser dead after timeout; will relaunch on next request"
    );
}

/// Try to create and configure a new tab. If the browser is dead, relaunch
/// Chrome once and retry. A generation counter ensures that only the first
/// thread to notice the failure pays the relaunch cost.
fn new_tab_or_relaunch(state: &Mutex<BrowserState>, user_agent: &str) -> Result<Arc<Tab>, Error> {
    // Fast path: browser is alive
    let (browser, snapshot_gen) = {
        let s = lock_unpoisoned(state);
        (s.browser.clone(), s.generation)
    };

    match setup_tab(&browser, user_agent) {
        Ok(tab) => return Ok(tab),
        Err(e) => tracing::warn!(error = %e, "tab creation failed, relaunching Chrome"),
    }

    // Slow path: relaunch
    let mut s = lock_unpoisoned(state);
    if s.generation == snapshot_gen {
        let new_browser = launch_browser()?;
        s.browser = Arc::new(new_browser);
        s.generation = s.generation.wrapping_add(1);
        tracing::info!(generation = s.generation, "Chrome relaunched successfully");
    }
    setup_tab(&s.browser, user_agent)
}

impl BrowserManager {
    pub fn launch() -> Result<Self, Error> {
        let browser = launch_browser()?;

        Ok(Self {
            state: Arc::new(Mutex::new(BrowserState {
                browser: Arc::new(browser),
                generation: 0,
            })),
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
        let state = self.state.clone();
        let ua = self.user_agent.clone();
        let cf_timeout = self.resolve_timeout(timeout_secs);
        let budget = self.total_budget(cf_timeout);

        self.with_budget(budget, async move {
            tokio::task::spawn_blocking(move || {
                let tab = new_tab_or_relaunch(&state, &ua)?;
                let _guard = TabGuard(tab.clone());
                navigate_and_wait_for_cf(&tab, &url, cf_timeout)?;
                tab.get_content()
                    .map_err(|e| Error::Extraction(e.to_string()))
            })
            .await?
        })
        .await
    }

    pub async fn screenshot_page(
        &self,
        url: String,
        timeout_secs: Option<u64>,
    ) -> Result<Vec<u8>, Error> {
        let state = self.state.clone();
        let ua = self.user_agent.clone();
        let cf_timeout = self.resolve_timeout(timeout_secs);
        let budget = self.total_budget(cf_timeout);

        self.with_budget(budget, async move {
            tokio::task::spawn_blocking(move || {
                let tab = new_tab_or_relaunch(&state, &ua)?;
                let _guard = TabGuard(tab.clone());
                navigate_and_wait_for_cf(&tab, &url, cf_timeout)?;
                tab.capture_screenshot(
                    headless_chrome::protocol::cdp::Page::CaptureScreenshotFormatOption::Png,
                    None,
                    None,
                    true,
                )
                .map_err(Error::Screenshot)
            })
            .await?
        })
        .await
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
        let state = self.state.clone();
        let ua = self.user_agent.clone();
        let cf_timeout = self.resolve_timeout(timeout_secs);
        let budget = self.total_budget(cf_timeout);

        self.with_budget(budget, async move {
            tokio::task::spawn_blocking(move || {
                let tab = new_tab_or_relaunch(&state, &ua)?;
                let _guard = TabGuard(tab.clone());
                navigate_and_wait_for_cf(&tab, &url, cf_timeout)?;

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
        })
        .await
    }

    fn resolve_timeout(&self, timeout_secs: Option<u64>) -> Duration {
        Duration::from_secs(timeout_secs.unwrap_or(self.default_timeout.as_secs()))
    }

    /// Total wall-clock budget for a tool call: the Cloudflare wait plus
    /// overhead for navigation, content extraction, and screenshotting. Hard-
    /// capped so a pathological request can't hold a worker forever.
    fn total_budget(&self, cf_timeout: Duration) -> Duration {
        const OVERHEAD: Duration = Duration::from_secs(20);
        const MAX: Duration = Duration::from_secs(60);
        (cf_timeout + OVERHEAD).min(MAX)
    }

    /// Enforce the budget. On timeout, mark the browser dead so the next
    /// request relaunches Chrome — a tab that stalled past the budget may
    /// have wedged the underlying Chrome process.
    async fn with_budget<F, T>(&self, budget: Duration, fut: F) -> Result<T, Error>
    where
        F: std::future::Future<Output = Result<T, Error>>,
    {
        match tokio::time::timeout(budget, fut).await {
            Ok(result) => result,
            Err(_) => {
                tracing::warn!(
                    budget_secs = budget.as_secs(),
                    "tool call exceeded budget; marking browser dead"
                );
                mark_browser_dead(&self.state);
                Err(Error::Timeout {
                    budget_secs: budget.as_secs(),
                })
            }
        }
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
