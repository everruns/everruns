// Parallel tool calls capability
//
// Controls whether the agent asks the provider to emit multiple tool calls per
// turn (and whether the local tool scheduler runs a batch concurrently):
//
// - `prefer`: explicitly request parallel tool calls. On providers that expose
//   a wire control (OpenAI/Anthropic families) this is sent on the request; the
//   local scheduler keeps its class-aware concurrent default. Lets the model
//   batch independent reads/searches instead of relying on each provider's
//   undocumented default.
// - `avoid`: ask the provider to emit at most one tool call per turn AND force
//   the local tool scheduler to serialize the batch. The local serialization
//   applies to every driver, so `avoid` is honored even on providers without a
//   wire control (Gemini/Bedrock).
// - `none`: no preference — omit the field and keep the provider default and the
//   scheduler's concurrent schedule. Same effect as not enabling the capability;
//   useful to neutralize an inherited preference from a parent harness.
//
// The resolved preference threads through `RuntimeAgent.parallel_tool_calls`
// into both the LLM request (provider-gated, see `ChatDriver::
// supports_parallel_tool_calls`) and `ActInput.parallel_tool_calls` (the local
// scheduler). An explicit `parallel_tool_calls` field on harness/agent/session
// is a lower-level escape hatch and takes precedence over this capability.

use everruns_core::capabilities::{Capability, CapabilityLocalization, SystemPromptContext};
use async_trait::async_trait;

/// Capability ID for the request-level parallel tool calls preference.
pub const PARALLEL_TOOL_CALLS_CAPABILITY_ID: &str = "parallel_tool_calls";

/// Resolved preference mode for the `parallel_tool_calls` capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelToolCallsMode {
    /// Request parallel tool calls (provider + concurrent scheduler).
    Prefer,
    /// Disable parallel tool calls (provider hint + serialized scheduler).
    Avoid,
    /// No preference — provider default and concurrent scheduler.
    None,
}

impl ParallelToolCallsMode {
    /// Parse a config string into a mode. Unknown values yield `None`.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "prefer" => Some(Self::Prefer),
            "avoid" => Some(Self::Avoid),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    /// Map the mode onto the request-level `parallel_tool_calls` preference
    /// carried by `RuntimeAgent`/`LlmCallConfig`/`ActInput`.
    ///
    /// `None` mode resolves to `None` (omit, provider default).
    pub fn to_preference(self) -> Option<bool> {
        match self {
            Self::Prefer => Some(true),
            Self::Avoid => Some(false),
            Self::None => None,
        }
    }
}

/// Resolve the `parallel_tool_calls` capability config into a request-level
/// preference.
///
/// - Capability present with no explicit `mode` (empty/`null` config) → `prefer`.
/// - Valid `mode` → that mode (`none` resolves to no preference).
/// - Malformed config (not an object) or an invalid/non-string `mode` → `None`,
///   so a bad runtime config neutralizes the capability rather than silently
///   enabling parallel tool calls. (`validate_config` already rejects these on
///   the write path; this is the defensive runtime fallback.)
pub fn parallel_tool_calls_from_config(config: &serde_json::Value) -> Option<bool> {
    if config.is_null() {
        return ParallelToolCallsMode::Prefer.to_preference();
    }
    let object = config.as_object()?;
    match object.get("mode") {
        None => ParallelToolCallsMode::Prefer.to_preference(),
        Some(serde_json::Value::String(mode)) => {
            ParallelToolCallsMode::parse(mode).and_then(ParallelToolCallsMode::to_preference)
        }
        Some(_) => None,
    }
}

/// Parallel tool calls capability.
///
/// Adds no tools or prompt text; it only configures the outbound LLM request
/// and the local tool scheduler.
pub struct ParallelToolCallsCapability;

#[async_trait]
impl Capability for ParallelToolCallsCapability {
    fn id(&self) -> &str {
        PARALLEL_TOOL_CALLS_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Parallel Tool Calls"
    }

    fn description(&self) -> &str {
        "Controls whether the agent requests multiple tool calls per turn and \
         runs them concurrently: prefer (request parallel), avoid (one at a \
         time, serialized), or none (provider default)."
    }

    fn category(&self) -> Option<&str> {
        Some("Optimization")
    }

    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "title": "Parallel tool calls",
                    "description": "prefer: request parallel tool calls (default); avoid: one tool call per turn, serialized locally; none: provider default.",
                    "enum": ["prefer", "avoid", "none"],
                    "default": "prefer"
                }
            }
        }))
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        if !config.is_object() {
            return Err("parallel_tool_calls config must be an object".to_string());
        }
        match config.get("mode") {
            None => Ok(()),
            Some(serde_json::Value::String(mode))
                if ParallelToolCallsMode::parse(mode).is_some() =>
            {
                Ok(())
            }
            Some(value) => Err(format!(
                "mode must be one of \"prefer\", \"avoid\", \"none\", got {value}"
            )),
        }
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some(
                    "Chooses whether the agent batches independent tool calls or runs them one at a time.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Паралельні виклики інструментів"),
                description: Some(
                    "Визначає, чи запитує агент кілька викликів інструментів за хід і чи виконує їх одночасно: prefer (запитувати паралельні), avoid (по одному, послідовно) або none (типова поведінка провайдера).",
                ),
                config_description: Some(
                    "Обирає, чи агент об'єднує незалежні виклики інструментів, чи виконує їх по одному.",
                ),
                config_overlay: Some(serde_json::json!({
                    "properties": {
                        "mode": {
                            "title": "Паралельні виклики інструментів",
                            "description": "prefer: запитувати паралельні виклики (типово); avoid: один виклик за хід, послідовно; none: типова поведінка провайдера."
                        }
                    }
                })),
            },
        ]
    }

    async fn system_prompt_contribution(&self, _ctx: &SystemPromptContext) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use everruns_core::capabilities::*;

    #[test]
    fn parse_known_modes() {
        assert_eq!(
            ParallelToolCallsMode::parse("prefer"),
            Some(ParallelToolCallsMode::Prefer)
        );
        assert_eq!(
            ParallelToolCallsMode::parse("avoid"),
            Some(ParallelToolCallsMode::Avoid)
        );
        assert_eq!(
            ParallelToolCallsMode::parse("none"),
            Some(ParallelToolCallsMode::None)
        );
        assert_eq!(ParallelToolCallsMode::parse("loud"), None);
    }

    #[test]
    fn mode_to_preference() {
        assert_eq!(ParallelToolCallsMode::Prefer.to_preference(), Some(true));
        assert_eq!(ParallelToolCallsMode::Avoid.to_preference(), Some(false));
        assert_eq!(ParallelToolCallsMode::None.to_preference(), None);
    }

    #[test]
    fn from_config_defaults_to_prefer() {
        // Capability present without explicit mode => prefer.
        assert_eq!(
            parallel_tool_calls_from_config(&serde_json::json!({})),
            Some(true)
        );
        assert_eq!(
            parallel_tool_calls_from_config(&serde_json::Value::Null),
            Some(true)
        );
    }

    #[test]
    fn from_config_honors_mode() {
        assert_eq!(
            parallel_tool_calls_from_config(&serde_json::json!({"mode": "prefer"})),
            Some(true)
        );
        assert_eq!(
            parallel_tool_calls_from_config(&serde_json::json!({"mode": "avoid"})),
            Some(false)
        );
        assert_eq!(
            parallel_tool_calls_from_config(&serde_json::json!({"mode": "none"})),
            None
        );
    }

    #[test]
    fn from_config_malformed_neutralizes() {
        // Invalid/non-string mode and non-object configs do not silently enable
        // parallel tool calls — they resolve to no preference.
        assert_eq!(
            parallel_tool_calls_from_config(&serde_json::json!({"mode": "loud"})),
            None
        );
        assert_eq!(
            parallel_tool_calls_from_config(&serde_json::json!({"mode": 5})),
            None
        );
        assert_eq!(
            parallel_tool_calls_from_config(&serde_json::json!([])),
            None
        );
    }

    #[test]
    fn validate_config_accepts_known_modes_only() {
        let cap = ParallelToolCallsCapability;
        assert!(cap.validate_config(&serde_json::Value::Null).is_ok());
        assert!(cap.validate_config(&serde_json::json!({})).is_ok());
        assert!(
            cap.validate_config(&serde_json::json!({"mode": "prefer"}))
                .is_ok()
        );
        assert!(
            cap.validate_config(&serde_json::json!({"mode": "avoid"}))
                .is_ok()
        );
        assert!(
            cap.validate_config(&serde_json::json!({"mode": "loud"}))
                .is_err()
        );
        assert!(cap.validate_config(&serde_json::json!([])).is_err());
    }

    #[test]
    fn localizations_resolve_uk() {
        let cap = ParallelToolCallsCapability;
        assert_eq!(
            cap.localized_name(Some("uk-UA")),
            "Паралельні виклики інструментів"
        );
    }
}
