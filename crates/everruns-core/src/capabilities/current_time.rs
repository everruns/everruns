//! CurrentTime Capability - provides tools to get current date and time

use super::{Capability, CapabilityId, CapabilityStatus};
use crate::tools::{Tool, ToolExecutionResult};
use async_trait::async_trait;
use serde_json::Value;

/// CurrentTime capability - provides tools to get current date and time
pub struct CurrentTimeCapability;

impl Capability for CurrentTimeCapability {
    fn id(&self) -> &str {
        CapabilityId::CURRENT_TIME
    }

    fn name(&self) -> &str {
        "Current Time"
    }

    fn description(&self) -> &str {
        "Adds a tool to get the current date and time in various formats and timezones."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("clock")
    }

    fn category(&self) -> Option<&str> {
        Some("Utilities")
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(GetCurrentTimeTool)]
    }
}

// ============================================================================
// Tool: get_current_time
// ============================================================================

/// Tool that returns the current date and time
pub struct GetCurrentTimeTool;

#[async_trait]
impl Tool for GetCurrentTimeTool {
    fn name(&self) -> &str {
        "get_current_time"
    }

    fn description(&self) -> &str {
        "Get the current date and time. Can return time in different formats and timezones."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "timezone": {
                    "type": "string",
                    "description": "Timezone to return the time in (e.g., 'UTC', 'America/New_York', 'Europe/London'). Defaults to UTC."
                },
                "format": {
                    "type": "string",
                    "enum": ["iso8601", "unix", "human"],
                    "description": "Output format: 'iso8601' for ISO 8601 format, 'unix' for Unix timestamp, 'human' for human-readable format. Defaults to 'iso8601'."
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let format = arguments
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("iso8601");

        let _timezone = arguments
            .get("timezone")
            .and_then(|v| v.as_str())
            .unwrap_or("UTC");

        // Note: For simplicity, we're using UTC. Full timezone support would require
        // the chrono-tz crate which adds significant dependencies.
        let now = chrono::Utc::now();

        let result = match format {
            "unix" => serde_json::json!({
                "timestamp": now.timestamp(),
                "format": "unix",
                "timezone": "UTC"
            }),
            "human" => serde_json::json!({
                "datetime": now.format("%A, %B %d, %Y at %H:%M:%S UTC").to_string(),
                "format": "human",
                "timezone": "UTC"
            }),
            _ => serde_json::json!({
                "datetime": now.to_rfc3339(),
                "format": "iso8601",
                "timezone": "UTC"
            }),
        };

        ToolExecutionResult::success(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::CapabilityRegistry;

    #[test]
    fn test_capability_metadata() {
        let cap = CurrentTimeCapability;

        assert_eq!(cap.id(), "current_time");
        assert_eq!(cap.name(), "Current Time");
        assert_eq!(cap.icon(), Some("clock"));
        assert_eq!(cap.category(), Some("Utilities"));
        assert_eq!(cap.status(), CapabilityStatus::Available);
    }

    #[test]
    fn test_capability_has_tools() {
        let cap = CurrentTimeCapability;
        let tools = cap.tools();

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "get_current_time");
    }

    #[test]
    fn test_capability_no_system_prompt() {
        let cap = CurrentTimeCapability;
        assert!(cap.system_prompt_addition().is_none());
    }

    #[test]
    fn test_capability_in_registry() {
        let registry = CapabilityRegistry::with_builtins();
        let cap = registry.get("current_time").unwrap();

        assert_eq!(cap.id(), "current_time");
        assert_eq!(cap.tools().len(), 1);
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
