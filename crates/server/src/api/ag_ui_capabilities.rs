//! `GET /v1/e/{endpoint_id}/ag-ui/capabilities`: the AG-UI 1.0
//! `AgentCapabilities` an endpoint declares.
//!
//! Decision: derived from the endpoint's channel config alone, never from the
//! agent's tools or prompts. It tells a consumer what the stream will carry
//! (reasoning, subagents, approval interrupts, usage), which the config
//! decides, and nothing a public caller could not learn by running the agent
//! (TM-API-026). The stream stays authoritative, as 1.0 says.

use crate::records::AgUiChannelConfig;
use everruns_core::ag_ui::{
    AgentCapabilities, HumanInTheLoopCapabilities, IdentityCapabilities, MultiAgentCapabilities,
    MultimodalCapabilities, MultimodalInputCapabilities, ReasoningCapabilities, ToolsCapabilities,
    TransportCapabilities,
};

/// `identity.type`: the platform powering the agent.
const IDENTITY_TYPE: &str = "everruns";

/// What an AG-UI endpoint declares, from its display identity and config.
pub(crate) fn capabilities(
    name: &str,
    description: Option<&str>,
    config: &AgUiChannelConfig,
) -> AgentCapabilities {
    AgentCapabilities {
        identity: Some(IdentityCapabilities {
            name: Some(name.to_string()),
            kind: Some(IDENTITY_TYPE.to_string()),
            description: description.map(str::to_string),
            ..IdentityCapabilities::default()
        }),
        transport: Some(TransportCapabilities {
            streaming: Some(true),
            ..TransportCapabilities::default()
        }),
        // The agent's own tools stay undeclared: names and schemas are
        // internal, and tool activity is the channel's fixed text.
        tools: Some(ToolsCapabilities {
            supported: Some(true),
            client_provided: Some(true),
            ..ToolsCapabilities::default()
        }),
        reasoning: Some(ReasoningCapabilities {
            supported: Some(config.reasoning_summary_visible),
            streaming: Some(config.reasoning_summary_visible),
            ..ReasoningCapabilities::default()
        }),
        // Undeclared unless the stream shows subagents: the agent may still
        // delegate, but a consumer would see none of it.
        multi_agent: config.subagents_visible.then(|| MultiAgentCapabilities {
            supported: Some(true),
            delegation: Some(true),
            ..MultiAgentCapabilities::default()
        }),
        multimodal: Some(MultimodalCapabilities {
            input: Some(MultimodalInputCapabilities {
                image: Some(true),
                ..MultimodalInputCapabilities::default()
            }),
            output: None,
        }),
        human_in_the_loop: Some(HumanInTheLoopCapabilities {
            supported: Some(true),
            interrupts: Some(true),
            approvals: Some(config.tool_approval_interrupts),
            ..HumanInTheLoopCapabilities::default()
        }),
        // Token usage on terminal events has no standard category.
        custom: Some(serde_json::Map::from_iter([(
            "everruns".to_string(),
            serde_json::json!({ "usage": config.usage_visible }),
        )])),
        ..AgentCapabilities::default()
    }
}
