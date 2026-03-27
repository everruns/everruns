//! Browserless Integration
//!
//! Cloud browser automation via Browserless REST API and CDP (WebSocket) sessions.
//! Supports screenshots, DOM reading, structured scraping, and multi-step
//! interactions (click, type, keyboard, mouse, touch).
//!
//! Two modes:
//! - **REST** (default): Each tool call uses a fresh ephemeral browser. No state, no cleanup.
//! - **CDP sessions**: `browserless_open_browser` opens a persistent browser via WebSocket.
//!   Subsequent tool calls reuse it (preserving login, cookies, navigation history).
//!   `browserless_close_browser` releases the browser.
//!
//! Decision: External integration crate, auto-registered via inventory plugin system
//! Decision: REST for one-shot operations, CDP WebSocket for persistent browser sessions
//! Decision: /function endpoint for multi-step interactions (click, type, navigate)
//! Decision: CDP sessions use Browserless.reconnect to keep browser alive between tool calls

pub mod cdp;
pub mod client;
pub mod connection;
pub mod session_tools;
pub mod state;
mod tools;
mod validation;

use everruns_core::LEASED_RESOURCES_FEATURE;
use everruns_core::capabilities::{Capability, CapabilityStatus, IntegrationPlugin, RiskLevel};
use everruns_core::connection_provider::ConnectionProviderPlugin;
use everruns_core::tools::Tool;

use connection::BrowserlessConnectionProvider;

use session_tools::{BrowserlessCloseBrowserTool, BrowserlessOpenBrowserTool};
use tools::{
    BrowserlessContentTool, BrowserlessInteractTool, BrowserlessNavigateTool,
    BrowserlessScrapeTool, BrowserlessScreenshotTool,
};

// ============================================================================
// Plugin Registration
// ============================================================================

inventory::submit! {
    IntegrationPlugin {
        experimental_only: false,
        factory: || Box::new(BrowserlessCapability),
    }
}

inventory::submit! {
    ConnectionProviderPlugin {
        experimental_only: true,
        factory: || Box::new(BrowserlessConnectionProvider),
    }
}

// ============================================================================
// Constants
// ============================================================================

const BROWSERLESS_API_BASE: &str = "https://production-sfo.browserless.io";
const BROWSERLESS_WS_BASE: &str = "wss://production-sfo.browserless.io";

/// Path for new CDP browser sessions. Browserless v2 requires `/chromium` —
/// the root path returns 400 Bad Request for WebSocket upgrades.
const BROWSERLESS_CDP_PATH: &str = "/chromium";

// ============================================================================
// BrowserlessCapability
// ============================================================================

pub struct BrowserlessCapability;

impl Capability for BrowserlessCapability {
    fn id(&self) -> &str {
        "browserless"
    }

    fn name(&self) -> &str {
        "Browserless"
    }

    fn description(&self) -> &str {
        "Cloud browser automation powered by Browserless. Take screenshots, read DOM content, \
         scrape structured data, and interact with web pages (click, type, keyboard, mouse, touch). \
         Supports persistent browser sessions via CDP for login flows. \
         Use cases: accessibility testing, regression testing, web scraping, UI validation."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Medium
    }

    fn icon(&self) -> Option<&str> {
        Some("browserless")
    }

    fn category(&self) -> Option<&str> {
        Some("Browser")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"## Browserless

Cloud browser automation via Browserless. Two modes of operation:

### Stateless Mode (default)
Each tool call uses a fresh browser session that is automatically destroyed after completion.
No resources are left behind.

### Persistent Session Mode (CDP)
Use `browserless_open_browser` to create a persistent browser session via Chrome DevTools Protocol.
The browser stays alive between tool calls, preserving login state, cookies, and navigation history.
Use `browserless_close_browser` when done to release the browser.
Persistent sessions also appear in the session Resources tab so users can see
what may be cleaned automatically after inactivity.

When a persistent session is active, the navigate/screenshot/content/interact tools will
automatically use it instead of creating fresh browsers.

Authentication: Browserless API token is resolved automatically from Settings > Connections > Browserless.
If not configured, guide the user to set up their token in Settings > Connections.

### Session Management Tools
- `browserless_open_browser` - Open a persistent browser session (stays alive between tool calls)
- `browserless_close_browser` - Close the persistent browser session and release resources

### Browser Tools
- `browserless_navigate` - Open a URL and get page metadata (title, links, headings, meta tags)
- `browserless_screenshot` - Take a PNG screenshot of a page (full page or specific element)
- `browserless_content` - Get the fully rendered HTML/DOM of a page (including JS-rendered content)
- `browserless_scrape` - Extract structured data using CSS selectors
- `browserless_interact` - Multi-step interactions: click, type, keyboard, mouse, touch, scroll, then capture result

### Typical Workflows

**One-shot (no persistent session needed):**
1. `browserless_navigate` to explore a page and discover its structure
2. `browserless_screenshot` to capture the visual state
3. `browserless_content` or `browserless_scrape` to read specific content

**Login-protected pages (use persistent session):**
1. `browserless_open_browser` with the login page URL
2. `browserless_interact` to fill in credentials and submit the form
3. `browserless_navigate` to browse authenticated pages
4. `browserless_screenshot` / `browserless_content` to inspect pages
5. `browserless_close_browser` when done

### Interaction Actions (browserless_interact)
- `click` - Click by CSS selector or x,y coordinates
- `type` - Type text into an input field (selector + value)
- `keyboard` - Press a key (Enter, Tab, Escape, etc.)
- `mouse_move` - Move mouse to x,y coordinates
- `touch` - Tap an element (mobile touch simulation)
- `scroll` - Scroll the page by a pixel amount
- `wait` - Wait for milliseconds
- `wait_for_selector` - Wait for a CSS selector to appear
- `navigate` - Navigate to a different URL mid-interaction

Set `return_screenshot: true` on `browserless_interact` to get a screenshot after all steps,
or leave it false to get the DOM content instead."#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(BrowserlessOpenBrowserTool),
            Box::new(BrowserlessCloseBrowserTool),
            Box::new(BrowserlessNavigateTool),
            Box::new(BrowserlessScreenshotTool),
            Box::new(BrowserlessContentTool),
            Box::new(BrowserlessScrapeTool),
            Box::new(BrowserlessInteractTool),
        ]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_storage"]
    }

    fn features(&self) -> Vec<&'static str> {
        vec![LEASED_RESOURCES_FEATURE]
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::capabilities::CapabilityStatus;

    #[test]
    fn test_capability_metadata() {
        let cap = BrowserlessCapability;
        assert_eq!(cap.id(), "browserless");
        assert_eq!(cap.name(), "Browserless");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("browserless"));
        assert_eq!(cap.category(), Some("Browser"));
    }

    #[test]
    fn test_cdp_path_is_set() {
        assert_eq!(BROWSERLESS_CDP_PATH, "/chromium");
        // New CDP sessions must connect to /chromium — root path returns 400
        let full_url = format!("{}{}", BROWSERLESS_WS_BASE, BROWSERLESS_CDP_PATH);
        assert_eq!(full_url, "wss://production-sfo.browserless.io/chromium");
    }

    #[test]
    fn test_capability_has_all_tools() {
        let cap = BrowserlessCapability;
        let tools = cap.tools();
        assert_eq!(tools.len(), 7);

        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"browserless_open_browser"));
        assert!(names.contains(&"browserless_close_browser"));
        assert!(names.contains(&"browserless_navigate"));
        assert!(names.contains(&"browserless_screenshot"));
        assert!(names.contains(&"browserless_content"));
        assert!(names.contains(&"browserless_scrape"));
        assert!(names.contains(&"browserless_interact"));
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = BrowserlessCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("browserless_open_browser"));
        assert!(prompt.contains("browserless_close_browser"));
        assert!(prompt.contains("browserless_navigate"));
        assert!(prompt.contains("browserless_screenshot"));
        assert!(prompt.contains("browserless_content"));
        assert!(prompt.contains("browserless_scrape"));
        assert!(prompt.contains("browserless_interact"));
        assert!(prompt.contains("Settings > Connections"));
        assert!(prompt.contains("Persistent Session Mode"));
        assert!(prompt.contains("CDP"));
    }

    #[test]
    fn test_all_tools_require_context() {
        let cap = BrowserlessCapability;
        for tool in cap.tools() {
            assert!(
                tool.requires_context(),
                "Tool {} should require context",
                tool.name()
            );
        }
    }

    #[test]
    fn test_capability_depends_on_session_storage() {
        let cap = BrowserlessCapability;
        assert_eq!(cap.dependencies(), vec!["session_storage"]);
    }

    #[test]
    fn test_capability_risk_level() {
        let cap = BrowserlessCapability;
        assert_eq!(cap.risk_level(), RiskLevel::Medium);
    }
}
