//! Payloads for capability usage, act and tool events.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use crate::localization::localized_tool_display_name;
use crate::typed_id::TurnId;

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

impl From<&crate::tool_types::ToolDefinition> for ToolDefinitionSummary {
    fn from(tool: &crate::tool_types::ToolDefinition) -> Self {
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
        tool_defs: &[crate::tool_types::ToolDefinition],
    ) -> Self {
        Self::with_definitions_and_locale(tool_calls, tool_defs, None)
    }

    pub fn with_definitions_and_locale(
        tool_calls: &[ToolCall],
        tool_defs: &[crate::tool_types::ToolDefinition],
        locale: Option<&str>,
    ) -> Self {
        let def_map: std::collections::HashMap<&str, &crate::tool_types::ToolDefinition> =
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
        tool_defs: &[crate::tool_types::ToolDefinition],
    ) -> Self {
        Self::with_definitions_and_locale(tool_calls, tool_defs, None)
    }

    pub fn with_definitions_and_locale(
        tool_calls: &[ToolCall],
        tool_defs: &[crate::tool_types::ToolDefinition],
        locale: Option<&str>,
    ) -> Self {
        let def_map: std::collections::HashMap<&str, &crate::tool_types::ToolDefinition> =
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
