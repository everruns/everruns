//! CurrentTime Capability - provides tools to get current date and time

use super::{Capability, CapabilityLocalization, CapabilityStatus, Fact, FactsContext};
use crate::tool_types::ToolHints;
use crate::tools::{Tool, ToolExecutionResult};
use async_trait::async_trait;
use chrono::SecondsFormat;
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
pub struct GetCurrentTimeTool;

#[async_trait]
impl Tool for GetCurrentTimeTool {
    fn narrate(
        &self,
        _tool_call: &crate::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(crate::tool_narration::narrate_current_time(phase, locale))
    }

    fn name(&self) -> &str {
        "get_current_time"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Get Current Time")
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

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let format = arguments
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("iso8601");

        let timezone = arguments
            .get("timezone")
            .and_then(|v| v.as_str())
            .unwrap_or("UTC");

        let timezone: chrono_tz::Tz = match timezone.parse() {
            Ok(timezone) => timezone,
            Err(_) => return ToolExecutionResult::tool_error("Unknown IANA timezone"),
        };
        let now = chrono::Utc::now().with_timezone(&timezone);

        let result = match format {
            "unix" => serde_json::json!({
                "timestamp": now.timestamp(),
                "format": "unix",
                "timezone": timezone.name()
            }),
            "human" => serde_json::json!({
                "datetime": now.format("%A, %B %d, %Y at %H:%M:%S %Z").to_string(),
                "format": "human",
                "timezone": timezone.name()
            }),
            _ => serde_json::json!({
                "datetime": now.to_rfc3339(),
                "format": "iso8601",
                "timezone": timezone.name()
            }),
        };

        ToolExecutionResult::success(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
    use serde_json::json;

    #[test]
    fn dynamic_fact_is_a_current_utc_instant_without_cached_prompt_text() {
        use crate::capabilities::Volatility;
        use crate::typed_id::SessionId;
        let before = Utc::now().timestamp();
        let facts = CurrentTimeCapability.facts(&Value::Null, &FactsContext::new(SessionId::new()));
        let after = Utc::now().timestamp();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].key, "current_time");
        assert_eq!(facts[0].volatility, Volatility::Dynamic);
        let instant = DateTime::parse_from_rfc3339(&facts[0].value).unwrap();
        assert_eq!(instant.offset().local_minus_utc(), 0);
        assert!((before..=after).contains(&instant.timestamp()));
        assert!(CurrentTimeCapability.system_prompt_addition().is_none());
    }

    #[tokio::test]
    async fn registered_time_tool_preserves_instant_across_formats_and_timezones() {
        let tools = CurrentTimeCapability.tools();
        assert_eq!(tools.len(), 1);
        let tool = &tools[0];
        for (zone, offset) in [
            ("UTC", 0),
            ("Asia/Kathmandu", 20700),
            ("America/Phoenix", -25200),
        ] {
            for format in ["iso8601", "unix", "human"] {
                let before = Utc::now().timestamp();
                let ToolExecutionResult::Success(value) =
                    tool.execute(json!({"timezone":zone,"format":format})).await
                else {
                    panic!("valid request failed")
                };
                let after = Utc::now().timestamp();
                let timestamp = match format {
                    "unix" => {
                        assert_eq!(
                            value,
                            json!({"timestamp":value["timestamp"],"format":"unix","timezone":zone})
                        );
                        value["timestamp"].as_i64().unwrap()
                    }
                    "iso8601" => {
                        let parsed =
                            DateTime::parse_from_rfc3339(value["datetime"].as_str().unwrap())
                                .unwrap();
                        assert_eq!(parsed.offset().local_minus_utc(), offset);
                        parsed.timestamp()
                    }
                    "human" => {
                        let text = value["datetime"].as_str().unwrap();
                        let parsed =
                            NaiveDateTime::parse_from_str(text, "%A, %B %d, %Y at %H:%M:%S %Z")
                                .unwrap();
                        let local = zone
                            .parse::<chrono_tz::Tz>()
                            .unwrap()
                            .from_local_datetime(&parsed)
                            .single()
                            .unwrap();
                        assert_eq!(
                            text,
                            local.format("%A, %B %d, %Y at %H:%M:%S %Z").to_string()
                        );
                        local.timestamp()
                    }
                    _ => unreachable!(),
                };
                assert!(
                    (before..=after).contains(&timestamp),
                    "{format} {zone}: {value}"
                );
                if format != "unix" {
                    assert_eq!(
                        value,
                        json!({"datetime":value["datetime"],"format":format,"timezone":zone})
                    );
                }
            }
        }
        let ToolExecutionResult::Success(default) = tool.execute(json!({})).await else {
            panic!("default failed")
        };
        assert_eq!(default["format"], "iso8601");
        assert_eq!(default["timezone"], "UTC");
        assert!(DateTime::parse_from_rfc3339(default["datetime"].as_str().unwrap()).is_ok());
        let ToolExecutionResult::ToolError(message) =
            tool.execute(json!({"timezone":"not/a-timezone"})).await
        else {
            panic!("unknown timezone must be rejected")
        };
        assert_eq!(message, "Unknown IANA timezone");
    }
}
