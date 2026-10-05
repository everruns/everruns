//! Payloads for capability usage, act and tool events.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use crate::runtime::localization::localized_tool_display_name;
use crate::runtime::typed_id::TurnId;

use super::*;

/// Reporting-only capability usage kinds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub enum CapabilityUsageKind {
    Configured,
    Resolved,
    Exposed,
    Invoked,
    EffectRan,
}

/// Single capability usage record. This intentionally carries only stable IDs
/// and small snapshots; prompts, messages, tool arguments, and results are not
/// allowed in reporting facts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CapabilityUsageRecord {
    /// Capability id (prefixed or namespaced) attributing this usage.
    pub capability_id: String,
    /// Capability display name for UI. `None` when the capability is unnamed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_name: Option<String>,
    /// Discriminator for the kind of usage being recorded (e.g. `tool_call`, `subagent_spawn`).
    pub usage_kind: CapabilityUsageKind,
    /// Concrete tool name when `usage_kind` is `tool_call`. `None` for non-tool usage kinds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Number of distinct usages recorded in this record (count-style records). `None` for duration-style records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_count: Option<u64>,
    /// Total wall-clock duration of the usage in milliseconds (duration-style records). `None` for count-style records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Data for capability.usage events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CapabilityUsageData {
    pub records: Vec<CapabilityUsageRecord>,
}

/// Summary of a tool call (compact form without arguments)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolCallSummary {
    pub id: String,
    pub name: String,
    /// Human-readable display name for UI rendering
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Human-readable narration for timeline rendering
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narration: Option<String>,
    /// Human-readable narration after the call completes
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_narration: Option<String>,
}

impl From<&ToolCall> for ToolCallSummary {
    fn from(tc: &ToolCall) -> Self {
        Self {
            id: tc.id.clone(),
            name: tc.name.clone(),
            display_name: None,
            narration: None,
            completed_narration: None,
        }
    }
}

/// Summary of a tool definition (compact form for events)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolDefinitionSummary {
    /// Tool name
    pub name: String,
    /// Human-readable display name for UI rendering
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Tool category for namespace grouping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Capability that contributed the tool definition, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_id: Option<String>,
    /// Human-readable capability name snapshot, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_name: Option<String>,
    /// Tool description
    pub description: String,
}

impl From<&crate::runtime::tool_types::ToolDefinition> for ToolDefinitionSummary {
    fn from(tool: &crate::runtime::tool_types::ToolDefinition) -> Self {
        let capability_attribution = tool.capability_attribution();
        Self {
            name: tool.name().to_string(),
            display_name: tool.display_name().map(|s| s.to_string()),
            category: tool.category().map(|s| s.to_string()),
            capability_id: capability_attribution.map(|(id, _)| id.to_string()),
            capability_name: capability_attribution.and_then(|(_, name)| name.map(str::to_string)),
            description: tool.description().to_string(),
        }
    }
}

/// Data for act.started event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ActStartedData {
    /// Tool calls to be executed
    pub tool_calls: Vec<ToolCallSummary>,
    /// Human-readable headline for the batch
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headline: Option<String>,
}

impl ActStartedData {
    pub fn new(tool_calls: &[ToolCall]) -> Self {
        Self::new_with_locale(tool_calls, None)
    }

    pub fn new_with_locale(tool_calls: &[ToolCall], locale: Option<&str>) -> Self {
        Self {
            tool_calls: tool_calls.iter().map(ToolCallSummary::from).collect(),
            headline: render_group_headline_with_locale(
                tool_calls,
                &[],
                ToolNarrationPhase::Started,
                locale,
            ),
        }
    }

    /// Create with display names resolved from tool definitions
    pub fn with_definitions(
        tool_calls: &[ToolCall],
        tool_defs: &[crate::runtime::tool_types::ToolDefinition],
    ) -> Self {
        Self::with_definitions_and_locale(tool_calls, tool_defs, None)
    }

    pub fn with_definitions_and_locale(
        tool_calls: &[ToolCall],
        tool_defs: &[crate::runtime::tool_types::ToolDefinition],
        locale: Option<&str>,
    ) -> Self {
        let def_map: std::collections::HashMap<&str, &crate::runtime::tool_types::ToolDefinition> =
            tool_defs.iter().map(|d| (d.name(), d)).collect();
        Self {
            tool_calls: tool_calls
                .iter()
                .map(|tc| {
                    let tool_def = def_map.get(tc.name.as_str()).copied();
                    let display_name = localized_tool_display_name(
                        &tc.name,
                        tool_def.and_then(|d| d.display_name()),
                        locale,
                    );
                    ToolCallSummary {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        display_name,
                        narration: Some(render_tool_narration_with_locale(
                            tool_def,
                            tc,
                            ToolNarrationPhase::Started,
                            locale,
                        )),
                        completed_narration: Some(render_tool_narration_with_locale(
                            tool_def,
                            tc,
                            ToolNarrationPhase::Completed,
                            locale,
                        )),
                    }
                })
                .collect(),
            headline: render_group_headline_with_locale(
                tool_calls,
                tool_defs,
                ToolNarrationPhase::Started,
                locale,
            ),
        }
    }
}

/// Data for act.completed event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ActCompletedData {
    /// Whether all tool calls completed
    pub completed: bool,

    /// Number of successful tool calls
    pub success_count: u32,

    /// Number of failed tool calls
    pub error_count: u32,

    /// Duration of the act phase in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Human-readable headline for the completed batch
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headline: Option<String>,
}

/// Data for tool.started event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolStartedData {
    /// The tool call being executed
    pub tool_call: ToolCall,
    /// Stable fingerprint of tool name + normalized arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_fingerprint: Option<String>,
    /// Human-readable display name for UI rendering
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Human-readable narration for timeline rendering
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narration: Option<String>,
}

/// Data for tool.completed event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolCompletedData {
    /// Tool call ID
    pub tool_call_id: String,

    /// Tool name
    pub tool_name: String,

    /// Stable fingerprint of tool name + normalized arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_fingerprint: Option<String>,

    /// Stable fingerprint of tool name + normalized result/error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_result_fingerprint: Option<String>,

    /// Human-readable display name for UI rendering
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    /// Whether the tool call succeeded
    pub success: bool,

    /// Status: "success", "error", "timeout", "cancelled"
    pub status: String,

    /// Result content (for successful calls)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Vec<ContentPart>>,

    /// Error message if failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Duration of the tool call in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,

    /// Capability that contributed the tool definition, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_id: Option<String>,

    /// Human-readable capability name snapshot, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_name: Option<String>,

    /// Human-readable narration for timeline rendering
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narration: Option<String>,

    /// Arguments the tool actually ran with, present only when `pre_tool_use`
    /// hooks rewrote the model-authored arguments (which `tool.started`
    /// carries). Values under credential-named keys (`password`, `*_token`,
    /// `authorization`, ...) read `[REDACTED]`. Bounded like the approval
    /// preview: past the budget this is a truncated JSON string and
    /// `executed_arguments_truncated` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executed_arguments: Option<serde_json::Value>,

    /// True when `executed_arguments` is a truncated preview.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub executed_arguments_truncated: bool,
}

impl ToolCompletedData {
    pub fn success(
        tool_call_id: String,
        tool_name: String,
        result: Vec<ContentPart>,
        duration_ms: Option<u64>,
    ) -> Self {
        Self {
            tool_call_id,
            tool_name,
            tool_call_fingerprint: None,
            tool_result_fingerprint: None,
            display_name: None,
            success: true,
            status: "success".to_string(),
            result: Some(result),
            error: None,
            duration_ms,
            capability_id: None,
            capability_name: None,
            narration: None,
            executed_arguments: None,
            executed_arguments_truncated: false,
        }
    }

    pub fn failure(
        tool_call_id: String,
        tool_name: String,
        status: String,
        error: String,
        duration_ms: Option<u64>,
    ) -> Self {
        Self {
            tool_call_id,
            tool_name,
            tool_call_fingerprint: None,
            tool_result_fingerprint: None,
            display_name: None,
            success: false,
            status,
            result: None,
            error: Some(error),
            duration_ms,
            capability_id: None,
            capability_name: None,
            narration: None,
            executed_arguments: None,
            executed_arguments_truncated: false,
        }
    }

    /// Set display name on this event data
    pub fn with_display_name(mut self, display_name: Option<String>) -> Self {
        self.display_name = display_name;
        self
    }

    pub fn with_fingerprints(
        mut self,
        tool_call_fingerprint: String,
        tool_result_fingerprint: String,
    ) -> Self {
        self.tool_call_fingerprint = Some(tool_call_fingerprint);
        self.tool_result_fingerprint = Some(tool_result_fingerprint);
        self
    }

    /// Set narration on this event data
    pub fn with_narration(mut self, narration: Option<String>) -> Self {
        self.narration = narration;
        self
    }

    /// Record the arguments the tool ran with, when hooks changed them.
    ///
    /// `authored` is what the model asked for, `executed` what was invoked.
    /// Equal arguments leave the event unchanged, so the field only appears
    /// when a hook rewrote the call. Values under credential-named keys are
    /// withheld and credential-looking string values are scrubbed: a hook may
    /// inject a credential the model never saw, and the event log is readable
    /// by everyone who can read the session.
    pub fn with_executed_arguments(
        mut self,
        authored: &serde_json::Value,
        executed: &serde_json::Value,
    ) -> Self {
        // Compare the raw values: scrubbing must not hide or invent a rewrite.
        if authored != executed {
            let mut redacted = redact_credential_fields(executed);
            // Scrub before truncating so the bound cannot split a secret into
            // a prefix the patterns no longer recognise.
            crate::secret_scrub::scrub_secrets_in_value(&mut redacted);
            let (preview, truncated) =
                crate::tool_approval_types::preview_tool_arguments(&redacted);
            self.executed_arguments = Some(preview);
            self.executed_arguments_truncated = truncated;
        }
        self
    }

    /// Set reporting attribution on this event data.
    pub fn with_capability_attribution(
        mut self,
        capability_id: Option<String>,
        capability_name: Option<String>,
    ) -> Self {
        self.capability_id = capability_id;
        self.capability_name = capability_name;
        self
    }
}

/// Placeholder for a value withheld from `executed_arguments`.
const REDACTED_ARGUMENT: &str = crate::secret_scrub::REDACTED;

/// THREAT[TM-HOOK-007]: copy of `value` with credential-named fields replaced
/// (value patterns are scrubbed separately by `secret_scrub`), so recording
/// what a hook rewrote does not publish what the hook injected.
fn redact_credential_fields(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(object) => serde_json::Value::Object(
            object
                .iter()
                .map(|(key, item)| {
                    let item = if crate::secret_scrub::is_credential_key(key) {
                        serde_json::Value::String(REDACTED_ARGUMENT.to_string())
                    } else {
                        redact_credential_fields(item)
                    };
                    (key.clone(), item)
                })
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(redact_credential_fields).collect())
        }
        other => other.clone(),
    }
}

/// Data for tool.progress event.
///
/// Emitted by tools during execution to report interim status updates.
/// This allows long-running tools (e.g., browser operations, sandbox setup)
/// to stream progress feedback between tool.started and tool.completed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolProgressData {
    /// Tool call ID this progress belongs to
    pub tool_call_id: String,

    /// Tool name
    pub tool_name: String,

    /// Human-readable status message (e.g., "Connecting to browser…")
    pub message: String,

    /// Human-readable display name for UI rendering
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Data for tool.output.delta event.
///
/// Emitted by tools during execution to stream incremental output chunks.
/// This enables live output rendering (e.g., bash stdout/stderr, command output)
/// between tool.started and tool.completed. Generic — usable by any tool that
/// produces streamed output (bashkit, Daytona exec, subagent speech, etc.).
///
/// The consumer accumulates deltas by tool_call_id for display. The final
/// tool.completed result is authoritative — deltas are informational only.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolOutputDeltaData {
    /// Tool call ID this output belongs to
    pub tool_call_id: String,

    /// Tool name
    pub tool_name: String,

    /// Incremental output chunk
    pub delta: String,

    /// Output stream identifier (e.g., "stdout", "stderr")
    pub stream: String,
}

/// Action taken during transcript repair for a dangling tool call.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TranscriptRepairAction {
    /// A settled result was found in durable storage and replayed into the transcript.
    Replay,
    /// A synthetic interrupted result was synthesized to make the transcript well-formed.
    Synthesize,
}

/// Data for transcript.repaired event (EVE-533).
///
/// Emitted once per dangling tool call when transcript repair runs before a `reason` call.
/// A dangling call is an assistant `tool_call` with no matching `ToolResult` in the
/// message history. Repair makes the transcript well-formed so the next LLM call succeeds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TranscriptRepairedData {
    /// The tool call ID that was repaired.
    pub tool_call_id: String,

    /// The tool name, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,

    /// Action taken: `replay` (settled result reused) or `synthesize` (interrupted placeholder added).
    pub action: TranscriptRepairAction,
}

/// Data for the `tool.call_repaired` event (EVE-600).
///
/// Emitted once per malformed tool call handled by the opt-in
/// `tool_call_repair` capability. `outcome` is the stable label
/// (`local-salvage` | `re-prompt` | `gave-up`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolCallRepairedData {
    /// Turn this repair belongs to.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// The tool call ID that was inspected/repaired.
    pub tool_call_id: String,

    /// The tool name the malformed call targeted.
    pub tool_name: String,

    /// Stable outcome label: `local-salvage`, `re-prompt`, or `gave-up`.
    pub outcome: String,
}

/// Data for the `tool.hosted_call` event (EVE-1115).
///
/// A provider-executed tool (OpenAI hosted `web_search`) changed state inside
/// the model's response. Informational: nothing was dispatched, and there is
/// no matching `tool.started` / `tool.completed` or tool-result message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct HostedToolCallData {
    /// Turn this call belongs to.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "turn_01933b5a00007000800000000000001"))]
    pub turn_id: TurnId,

    /// Provider item id; stable across the call's state changes.
    pub call_id: String,

    /// Hosted tool name, e.g. `web_search`.
    pub tool_name: String,

    /// `in_progress`, `completed`, or `failed`.
    pub status: String,

    /// Short detail once known, e.g. the search query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// Data for tool.call_requested event
///
/// Emitted when the agent needs client-side tool calls executed.
/// The workflow pauses until the client submits results via the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolCallRequestedData {
    /// Tool calls that need to be executed by the client
    pub tool_calls: Vec<ToolCall>,
    /// Optional summaries with display names and narration for UI rendering
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_summaries: Vec<ToolCallSummary>,
    /// Human-readable headline for the requested batch
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headline: Option<String>,
    /// Human-readable headline after the requested batch completes
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_headline: Option<String>,
}

impl ToolCallRequestedData {
    pub fn with_definitions(
        tool_calls: &[ToolCall],
        tool_defs: &[crate::runtime::tool_types::ToolDefinition],
    ) -> Self {
        Self::with_definitions_and_locale(tool_calls, tool_defs, None)
    }

    pub fn with_definitions_and_locale(
        tool_calls: &[ToolCall],
        tool_defs: &[crate::runtime::tool_types::ToolDefinition],
        locale: Option<&str>,
    ) -> Self {
        let def_map: std::collections::HashMap<&str, &crate::runtime::tool_types::ToolDefinition> =
            tool_defs.iter().map(|d| (d.name(), d)).collect();

        let tool_summaries = tool_calls
            .iter()
            .map(|tool_call| {
                let tool_def = def_map.get(tool_call.name.as_str()).copied();
                ToolCallSummary {
                    id: tool_call.id.clone(),
                    name: tool_call.name.clone(),
                    display_name: localized_tool_display_name(
                        &tool_call.name,
                        tool_def.and_then(|def| def.display_name()),
                        locale,
                    ),
                    narration: Some(render_tool_narration_with_locale(
                        tool_def,
                        tool_call,
                        ToolNarrationPhase::Waiting,
                        locale,
                    )),
                    completed_narration: Some(render_tool_narration_with_locale(
                        tool_def,
                        tool_call,
                        ToolNarrationPhase::Completed,
                        locale,
                    )),
                }
            })
            .collect();

        Self {
            tool_calls: tool_calls.to_vec(),
            tool_summaries,
            headline: render_group_headline_with_locale(
                tool_calls,
                tool_defs,
                ToolNarrationPhase::Waiting,
                locale,
            ),
            completed_headline: render_group_headline_with_locale(
                tool_calls,
                tool_defs,
                ToolNarrationPhase::Completed,
                locale,
            ),
        }
    }
}

// ============================================================================
// LLM Event Data Types
// ============================================================================

#[cfg(test)]
mod executed_arguments_tests {
    use super::*;
    use serde_json::json;

    fn completed() -> ToolCompletedData {
        ToolCompletedData::success("call_1".into(), "http".into(), Vec::new(), None)
    }

    #[test]
    fn unchanged_arguments_are_not_recorded() {
        let args = json!({ "url": "https://example.com" });
        let data = completed().with_executed_arguments(&args, &args);
        assert_eq!(data.executed_arguments, None);
        let wire = serde_json::to_value(&data).unwrap();
        assert!(wire.get("executed_arguments").is_none());
        assert!(wire.get("executed_arguments_truncated").is_none());
    }

    #[test]
    fn hook_injected_credentials_are_withheld() {
        let authored = json!({ "url": "https://example.com", "max_tokens": 5 });
        let executed = json!({
            "url": "https://example.com",
            "max_tokens": 5,
            "headers": { "Authorization": "Bearer abc", "X-Api-Key": "k" },
            "auth": [{ "access_token": "t", "client_secret": "s" }],
            "password": "p",
        });
        let data = completed().with_executed_arguments(&authored, &executed);
        assert_eq!(
            data.executed_arguments,
            Some(json!({
                "url": "https://example.com",
                "max_tokens": 5,
                "headers": { "Authorization": "[REDACTED]", "X-Api-Key": "[REDACTED]" },
                "auth": [{ "access_token": "[REDACTED]", "client_secret": "[REDACTED]" }],
                "password": "[REDACTED]",
            }))
        );
        assert!(!data.executed_arguments_truncated);
    }

    #[test]
    fn hook_injected_secrets_under_innocuous_keys_are_scrubbed() {
        let authored = json!({ "cmd": "curl https://api.example.com" });
        let executed = json!({
            "cmd": "curl -H 'Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig' https://api.example.com",
            "env": ["OPENAI=sk-live0123456789abcdefXYZ", "AWS=AKIAABCDEFGHIJKLMNOP"],
            "steps": [{ "remote": "https://bot:hunter2@git.example.com/o/r.git" }],
            "note": "plain text stays",
        });
        let data = completed().with_executed_arguments(&authored, &executed);
        assert_eq!(
            data.executed_arguments,
            Some(json!({
                "cmd": "curl -H 'Authorization: [REDACTED]' https://api.example.com",
                "env": ["OPENAI=[REDACTED]", "AWS=[REDACTED]"],
                "steps": [{ "remote": "https://[REDACTED]@git.example.com/o/r.git" }],
                "note": "plain text stays",
            }))
        );
    }

    #[test]
    fn secret_free_rewrite_is_recorded_verbatim() {
        let authored = json!({ "url": "https://example.com" });
        let executed = json!({ "url": "https://example.com", "timeout_ms": 500, "tag": "a@b" });
        let data = completed().with_executed_arguments(&authored, &executed);
        assert_eq!(data.executed_arguments, Some(executed));
    }

    #[test]
    fn rewrite_that_only_swaps_a_secret_is_still_recorded() {
        // Both scrub to the same text, so detection must compare raw values.
        let authored = json!({ "cmd": "use sk-aaaaaaaaaaaaaaaaaaaa" });
        let executed = json!({ "cmd": "use sk-bbbbbbbbbbbbbbbbbbbb" });
        let data = completed().with_executed_arguments(&authored, &executed);
        assert_eq!(
            data.executed_arguments,
            Some(json!({ "cmd": "use [REDACTED]" }))
        );
    }

    #[test]
    fn secrets_are_scrubbed_before_truncation() {
        use crate::tool_approval_types::TOOL_ARGUMENTS_PREVIEW_BYTES;
        let key = "sk-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        // Serialized as `{"a":"<pad> <key>"}`: the preview bound falls 8 bytes
        // into the key, leaving `sk-` plus fewer than the 16 chars the pattern
        // needs, so truncating first would publish an unscrubbable prefix.
        let prefix = r#"{"a":""#.len();
        let pad = "x".repeat(TOOL_ARGUMENTS_PREVIEW_BYTES - prefix - 1 - 8);
        let executed = json!({ "a": format!("{pad} {key}") });
        let data = completed().with_executed_arguments(&json!({}), &executed);
        assert!(data.executed_arguments_truncated);
        let preview = data.executed_arguments.unwrap();
        let preview = preview.as_str().unwrap();
        assert!(
            !preview.contains("sk-AAAA"),
            "secret prefix leaked: {}",
            &preview[preview.len() - 16..]
        );
        assert!(preview.ends_with(" [REDACTE"));
    }
}
