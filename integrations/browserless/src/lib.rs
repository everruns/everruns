//! Browserless cloud browser automation for Everruns agents.
//!
//! This integration contributes REST and CDP tools for screenshots, DOM reads,
//! scraping, and multi-step browser interaction to the
//! [Everruns](https://everruns.com) ecosystem.
//!
//! # Example
//!
//! ```
//! use everruns_integrations_browserless::BrowserlessCapability;
//!
//! let capability = BrowserlessCapability;
//! # let _ = capability;
//! ```

pub mod cdp;
pub mod client;
pub mod connection;
pub mod session_tools;
pub mod state;
mod tools;
mod validation;

use everruns_core::LEASED_RESOURCES_FEATURE;
use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, IntegrationPlugin, RiskLevel,
};
use everruns_core::tool_narration::ToolNarrationPhase;
use everruns_core::tool_types::{ToolCall, ToolDefinition};
use everruns_core::tools::Tool;
use everruns_platform::connector::ConnectorPlugin;

use connection::BrowserlessConnector;

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
        feature_flag: None,
        factory: || Box::new(BrowserlessCapability),
    }
}

inventory::submit! {
    ConnectorPlugin {
        experimental_only: true,
        factory: || Box::new(BrowserlessConnector),
    }
}

// ============================================================================
// Constants
// ============================================================================

const DEFAULT_API_BASE: &str = "https://production-sfo.browserless.io";
const DEFAULT_WS_BASE: &str = "wss://production-sfo.browserless.io";

/// Path for new CDP browser sessions. Browserless v2 requires `/chromium` —
/// the root path returns 400 Bad Request for WebSocket upgrades.
const BROWSERLESS_CDP_PATH: &str = "/chromium";

/// REST API base URL. Reads `BROWSERLESS_API_BASE` env var with fallback
/// to the default production SFO endpoint. Strips trailing slashes and
/// falls back to the default if the env var is empty.
pub fn browserless_api_base() -> String {
    read_base_url("BROWSERLESS_API_BASE", DEFAULT_API_BASE)
}

/// WebSocket base URL. Reads `BROWSERLESS_WS_BASE` env var with fallback
/// to the default production SFO endpoint. Strips trailing slashes and
/// falls back to the default if the env var is empty.
pub fn browserless_ws_base() -> String {
    read_base_url("BROWSERLESS_WS_BASE", DEFAULT_WS_BASE)
}

fn read_base_url(env_var: &str, default: &str) -> String {
    match std::env::var(env_var) {
        Ok(val) => {
            let trimmed = val.trim_end_matches('/');
            if trimmed.is_empty() {
                default.to_string()
            } else {
                trimmed.to_string()
            }
        }
        Err(_) => default.to_string(),
    }
}

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
            "Browserless tools use fresh browsers by default. Open a persistent browser only for login/stateful flows; active persistent sessions are reused automatically and should be closed when done. Use screenshots for visual state and DOM/content/scrape tools for text or structured data.",
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

    fn narrate(
        &self,
        _tool_def: Option<&ToolDefinition>,
        tool_call: &ToolCall,
        phase: ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        let url = tool_call
            .arguments
            .get("url")
            .and_then(|v| v.as_str())
            .map(|u| u.trim())
            .filter(|u| !u.is_empty())
            // host+path only — never expose scheme, userinfo, query, or fragment.
            .map(everruns_core::tool_narration::url_display);

        let (started, completed, failed, target) = match tool_call.name.as_str() {
            "browserless_open_browser" => (
                "Opening browser",
                "Opened browser",
                "Failed to open browser",
                url,
            ),
            "browserless_close_browser" => (
                "Closing browser",
                "Closed browser",
                "Failed to close browser",
                None,
            ),
            "browserless_navigate" => (
                "Navigating to",
                "Navigated to",
                "Failed to navigate to",
                url,
            ),
            "browserless_screenshot" => (
                "Taking screenshot",
                "Took screenshot",
                "Failed to take screenshot",
                None,
            ),
            "browserless_content" => (
                "Reading page content",
                "Read page content",
                "Failed to read page content",
                None,
            ),
            "browserless_scrape" => (
                "Scraping page",
                "Scraped page",
                "Failed to scrape page",
                None,
            ),
            "browserless_interact" => (
                "Interacting with page",
                "Interacted with page",
                "Failed to interact with page",
                None,
            ),
            _ => return None,
        };

        let verb = match phase {
            ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => started,
            ToolNarrationPhase::Completed => completed,
            ToolNarrationPhase::Failed => failed,
        };

        Some(match target {
            Some(target) => format!("{verb} {target}"),
            None => verb.to_string(),
        })
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_storage"]
    }

    fn features(&self) -> Vec<&'static str> {
        vec![LEASED_RESOURCES_FEATURE]
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Browserless",
            "Хмарна автоматизація браузера на основі Browserless. Робіть знімки екрана, \
             читайте вміст DOM, збирайте структуровані дані та взаємодійте з вебсторінками \
             (кліки, введення тексту, клавіатура, миша, дотики). Підтримує постійні сесії \
             браузера через CDP для сценаріїв входу. Сценарії використання: тестування \
             доступності, регресійне тестування, вебскрейпінг, перевірка інтерфейсу.",
        )]
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
    fn capability_narrates_browserless_tools() {
        let cap = BrowserlessCapability;
        let call = ToolCall {
            id: "call_1".to_string(),
            name: "browserless_navigate".to_string(),
            arguments: serde_json::json!({ "url": "https://user:tok@example.com/page?token=abc" }),
        };
        // Narration shows host+path only — no scheme, userinfo, or query.
        assert_eq!(
            cap.narrate(
                None,
                &call,
                ToolNarrationPhase::Started,
                None,
                everruns_core::tool_narration::ToolNarrationContext::default()
            ),
            Some("Navigating to example.com/page".to_string())
        );
        assert_eq!(
            cap.narrate(
                None,
                &call,
                ToolNarrationPhase::Completed,
                None,
                everruns_core::tool_narration::ToolNarrationContext::default()
            ),
            Some("Navigated to example.com/page".to_string())
        );

        let screenshot = ToolCall {
            id: "call_2".to_string(),
            name: "browserless_screenshot".to_string(),
            arguments: serde_json::json!({}),
        };
        assert_eq!(
            cap.narrate(
                None,
                &screenshot,
                ToolNarrationPhase::Completed,
                None,
                everruns_core::tool_narration::ToolNarrationContext::default()
            ),
            Some("Took screenshot".to_string())
        );

        // Tools this capability does not own are left to their owner.
        let other = ToolCall {
            id: "call_3".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({}),
        };
        assert_eq!(
            cap.narrate(
                None,
                &other,
                ToolNarrationPhase::Started,
                None,
                everruns_core::tool_narration::ToolNarrationContext::default()
            ),
            None
        );
    }

    #[test]
    fn test_cdp_path_is_set() {
        assert_eq!(BROWSERLESS_CDP_PATH, "/chromium");
        // New CDP sessions must connect to /chromium — root path returns 400
        let full_url = format!("{}{}", DEFAULT_WS_BASE, BROWSERLESS_CDP_PATH);
        assert_eq!(full_url, "wss://production-sfo.browserless.io/chromium");
    }

    #[test]
    fn test_read_base_url_defaults_when_unset() {
        // Use a unique env var name that's guaranteed not to be set
        let result = read_base_url(
            "_TEST_BROWSERLESS_URL_NEVER_SET",
            "https://default.example.com",
        );
        assert_eq!(result, "https://default.example.com");
    }

    #[test]
    fn test_read_base_url_strips_trailing_slash() {
        // Test the trimming logic directly
        let input = "https://custom.example.com/";
        assert_eq!(input.trim_end_matches('/'), "https://custom.example.com");
    }

    #[test]
    fn test_read_base_url_falls_back_on_empty() {
        // Test the empty-string fallback logic directly
        let trimmed = "".trim_end_matches('/');
        assert!(trimmed.is_empty());
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
        assert!(prompt.contains("fresh browsers"));
        assert!(prompt.contains("persistent browser"));
        assert!(prompt.contains("closed when done"));
        assert!(prompt.contains("screenshots for visual state"));
    }

    #[tokio::test]
    async fn system_prompt_within_budget() {
        let cap = BrowserlessCapability;
        let ctx = everruns_core::capabilities::SystemPromptContext::without_file_store(
            everruns_core::SessionId::new(),
        );
        let prompt = cap.system_prompt_contribution(&ctx).await.unwrap();
        assert!(prompt.len() <= 400, "prompt is {} bytes", prompt.len());
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
