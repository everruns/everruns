//! Browserless Integration
//!
//! Cloud browser automation via Browserless REST API.
//! Supports screenshots, DOM reading, structured scraping, and multi-step
//! interactions (click, type, keyboard, mouse, touch).
//!
//! Decision: External integration crate, auto-registered via inventory plugin system
//! Decision: REST-only (no WebSocket/Puppeteer sessions). Each call spins up a fresh
//!   browser and tears it down — no resources left behind.
//! Decision: /function endpoint for multi-step interactions (click, type, navigate)
//! Decision: No persistent browser sessions to manage = no state to track per session

pub mod client;
pub mod connection;
pub mod state;
mod tools;

use everruns_core::capabilities::{Capability, CapabilityStatus, IntegrationPlugin, RiskLevel};
use everruns_core::connection_provider::ConnectionProviderPlugin;
use everruns_core::tools::Tool;

use connection::BrowserlessConnectionProvider;

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

Cloud browser automation via Browserless. Each tool call uses a fresh browser session that is
automatically destroyed after completion — no resources are left behind.

Authentication: Browserless API token is resolved automatically from Settings > Connections > Browserless.
If not configured, guide the user to set up their token in Settings > Connections.

Tools:
- `browserless_navigate` - Open a URL and get page metadata (title, links, headings, meta tags)
- `browserless_screenshot` - Take a PNG screenshot of a page (full page or specific element)
- `browserless_content` - Get the fully rendered HTML/DOM of a page (including JS-rendered content)
- `browserless_scrape` - Extract structured data using CSS selectors
- `browserless_interact` - Multi-step interactions: click, type, keyboard, mouse, touch, scroll, then capture result

Typical workflow:
1. `browserless_navigate` to explore a page and discover its structure
2. `browserless_screenshot` to capture the visual state
3. `browserless_content` or `browserless_scrape` to read specific content
4. `browserless_interact` for multi-step flows (login, form filling, menu navigation)

The `browserless_interact` tool supports these actions in its `steps` array:
- `click` - Click an element by CSS selector or x,y coordinates
- `type` - Type text into an input field (selector + value)
- `keyboard` - Press a key (Enter, Tab, Escape, etc.)
- `mouse_move` - Move mouse to x,y coordinates
- `touch` - Tap an element (mobile touch simulation)
- `scroll` - Scroll the page by a pixel amount
- `wait` - Wait for a specified number of milliseconds
- `wait_for_selector` - Wait for a CSS selector to appear
- `navigate` - Navigate to a different URL mid-interaction

Set `return_screenshot: true` on `browserless_interact` to get a screenshot after all steps,
or leave it false to get the DOM content instead."#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(BrowserlessNavigateTool),
            Box::new(BrowserlessScreenshotTool),
            Box::new(BrowserlessContentTool),
            Box::new(BrowserlessScrapeTool),
            Box::new(BrowserlessInteractTool),
        ]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec![]
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
    fn test_capability_has_all_tools() {
        let cap = BrowserlessCapability;
        let tools = cap.tools();
        assert_eq!(tools.len(), 5);

        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
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
        assert!(prompt.contains("browserless_navigate"));
        assert!(prompt.contains("browserless_screenshot"));
        assert!(prompt.contains("browserless_content"));
        assert!(prompt.contains("browserless_scrape"));
        assert!(prompt.contains("browserless_interact"));
        assert!(prompt.contains("Settings > Connections"));
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
    fn test_capability_no_dependencies() {
        let cap = BrowserlessCapability;
        assert!(cap.dependencies().is_empty());
    }

    #[test]
    fn test_capability_risk_level() {
        let cap = BrowserlessCapability;
        assert_eq!(cap.risk_level(), RiskLevel::Medium);
    }
}
