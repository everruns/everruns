use serde::{Deserialize, Serialize};

use crate::ag_ui::{Metadata, Tool};

/// What an agent declares it can do. Omitted means undeclared; the stream is
/// authoritative. How a consumer obtains a declaration is left to the
/// implementation.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<IdentityCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<TransportCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<OutputCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<StateCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multi_agent: Option<MultiAgentCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multimodal: Option<MultimodalCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<ExecutionCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_in_the_loop: Option<HumanInTheLoopCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documentation_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransportCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streaming: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_binary: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_notifications: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumable: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supported: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<Tool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_provided: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_output: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supported_mime_types: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshots: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deltas: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persistent_state: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiAgentCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supported: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoffs: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagents: Option<Vec<SubagentInfo>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentInfo {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supported: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streaming: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultimodalCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<MultimodalInputCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<MultimodalOutputCapabilities>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultimodalInputCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pdf: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultimodalOutputCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_execution: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandboxed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<u64>,
    /// Milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_execution_time: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanInTheLoopCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supported: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approvals: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interventions: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interrupts: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approve_with_edits: Option<bool>,
}
