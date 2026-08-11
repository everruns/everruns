//! CurrentTime Capability - provides tools to get current date and time

use chrono::SecondsFormat;
use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, Fact, FactsContext,
};
use everruns_core::tools::Tool;
use serde_json::Value;

pub const CURRENT_TIME_CAPABILITY_ID: &str = "current_time";

/// CurrentTime capability - provides tools to get current date and time
pub struct CurrentTimeCapability;

impl Capability for CurrentTimeCapability {
    fn id(&self) -> &str {
        CURRENT_TIME_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Current Time"
    }

    fn description(&self) -> &str {
        "Adds a tool to get the current date and time in various formats and timezones."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Поточний час",
            "Додає інструмент для отримання поточної дати й часу в різних форматах і часових поясах.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("clock")
    }

    fn category(&self) -> Option<&str> {
        Some("Core")
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(GetCurrentTimeTool)]
    }

    /// Contribute the current UTC time as a dynamic fact. The runtime appends
    /// it to a live `<facts>` block at the conversation tail each turn, so the
    /// model always knows "now" without a tool round-trip and without the
    /// changing value invalidating the system-prompt cache. The
    /// `get_current_time` tool remains for explicit timezone/format queries.
    fn facts(&self, _config: &Value, _ctx: &FactsContext) -> Vec<Fact> {
        let now = chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        vec![Fact::dynamic("current_time", now)]
    }
}

// ============================================================================
// Tool: get_current_time
// ============================================================================

/// Tool that returns the current date and time
// EVE-884: the tool itself is part of the kernel default tool set, so it
// lives in core; this capability advertises and documents it.
pub use everruns_core::default_tools::GetCurrentTimeTool;

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::tools::{Tool, ToolExecutionResult};

    // Metadata/tool-list constants covered by builtin_capabilities_satisfy_registry_invariants.

    #[test]
    fn test_capability_no_system_prompt() {
        let cap = CurrentTimeCapability;
        assert!(cap.system_prompt_addition().is_none());
    }

    #[test]
    fn test_contributes_dynamic_current_time_fact() {
        use everruns_core::capabilities::{FactsContext, Volatility};
        use everruns_core::typed_id::SessionId;

        let cap = CurrentTimeCapability;
        let facts = cap.facts(
            &serde_json::Value::Null,
            &FactsContext::new(SessionId::new()),
        );
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].key, "current_time");
        assert_eq!(facts[0].volatility, Volatility::Dynamic);
        // RFC3339 UTC, e.g. 2026-07-04T12:00:00Z
        assert!(facts[0].value.ends_with('Z'), "got: {}", facts[0].value);
        assert!(facts[0].value.contains('T'));
    }

    #[tokio::test]
    async fn test_get_current_time_iso8601() {
        let tool = GetCurrentTimeTool;
        let result = tool.execute(serde_json::json!({})).await;

        if let ToolExecutionResult::Success(value) = result {
            assert!(value.get("datetime").is_some());
            assert_eq!(value.get("format").unwrap().as_str().unwrap(), "iso8601");
            assert_eq!(value.get("timezone").unwrap().as_str().unwrap(), "UTC");
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_get_current_time_unix() {
        let tool = GetCurrentTimeTool;
        let result = tool.execute(serde_json::json!({"format": "unix"})).await;

        if let ToolExecutionResult::Success(value) = result {
            assert!(value.get("timestamp").is_some());
            assert_eq!(value.get("format").unwrap().as_str().unwrap(), "unix");
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_get_current_time_human() {
        let tool = GetCurrentTimeTool;
        let result = tool.execute(serde_json::json!({"format": "human"})).await;

        if let ToolExecutionResult::Success(value) = result {
            assert!(value.get("datetime").is_some());
            assert_eq!(value.get("format").unwrap().as_str().unwrap(), "human");
            let datetime = value.get("datetime").unwrap().as_str().unwrap();
            assert!(datetime.contains("at"));
        } else {
            panic!("Expected success");
        }
    }
}
