//! Agent Execution API wire types.
//!
//! The execution API is everything a caller needs to talk to one agent: its
//! card, sessions, messages, events and answers to questions and approvals.
//! It is rooted at an *agent base URL* and every route is relative to it:
//! `/v1/channels/{channel_id}` on the everruns server, `/v1/channels/{agent}`
//! in a serve app. Both hosts serve the same shapes from these types, so the
//! agent client in the SDK works against either.
//!
//! Only types the two hosts did not already share live here. Session, message
//! and event shapes stay the server's `/v1` shapes, which serve mirrors.

use serde::{Deserialize, Serialize};

/// What a caller learns from `GET {agent base URL}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AgentCard {
    /// Display name of the agent.
    #[cfg_attr(feature = "openapi", schema(example = "Support agent"))]
    pub name: String,
    /// What the agent is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Session events can be followed live over SSE.
    pub streaming: bool,
    /// Which message content the agent accepts.
    pub input: AgentCardInput,
    /// Credentials the agent base URL accepts. Empty means anonymous.
    #[serde(default)]
    pub auth: Vec<AgentCardAuth>,
    /// Prompts a client may offer to start a conversation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conversation_starters: Vec<String>,
    /// Where to go next.
    pub links: AgentCardLinks,
}

/// Message content an agent accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AgentCardInput {
    pub text: bool,
    pub images: bool,
    pub files: bool,
}

impl AgentCardInput {
    /// Text only.
    pub const TEXT: Self = Self {
        text: true,
        images: false,
        files: false,
    };
}

/// One accepted credential kind. Mirrors the HTTP auth scheme a client sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentCardAuth {
    /// `Authorization: Bearer <agent key>`.
    AgentKey,
    /// `Authorization: Bearer <JWT>` issued by an OpenID Connect provider.
    Oidc {
        /// Issuer the token must come from.
        issuer: String,
    },
    /// `Authorization: Bearer <opaque token>` checked by OAuth 2.0
    /// introspection (RFC 7662).
    OAuth2,
    /// A short-lived runtime token from the agent's `/runtime-auth` exchange.
    RuntimeToken,
    /// `Authorization: Bearer <personal access token>` of a member of the
    /// organization that owns the agent.
    PersonalAccessToken,
}

/// Links from an agent card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AgentCardLinks {
    /// The session collection: create with `POST`, list yours with `GET`.
    pub sessions: String,
    /// The same agent's AG-UI endpoint, when one is live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ag_ui: Option<String>,
    /// The same agent's A2A Agent Card, when one is live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a2a: Option<String>,
}

/// `POST {agent base URL}/sessions`. The URL names the agent, so there is no
/// agent or harness field.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateAgentSessionRequest {
    /// Optional title for the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Caller-owned key/value data kept with the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub metadata: Option<serde_json::Value>,
}

/// What a person decided about one gated tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ToolApprovalDecision {
    /// Run this call, once.
    Allow,
    /// Run this call, and every later call of the same tool in this session.
    AllowAlways,
    /// Do not run this call.
    Reject,
    /// Do not run this call, nor any later call of the same tool in this session.
    RejectAlways,
}

impl ToolApprovalDecision {
    /// Whether the call runs.
    pub fn allows(self) -> bool {
        matches!(self, Self::Allow | Self::AllowAlways)
    }
}

/// One decision in a submission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ToolApprovalAnswer {
    /// The pending approval being answered.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "tool_approval_toolu_01933b5a00007000800000000000001")
    )]
    pub tool_call_id: String,
    /// The person's decision.
    pub decision: ToolApprovalDecision,
}

/// Request to answer pending tool-approval requests, `POST …/sessions/{id}/tool-approvals`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SubmitToolApprovalsRequest {
    /// Decisions for the pending requests. A pending request in the same batch
    /// that is left out is resolved as not approved: the turn resumes once, so
    /// every request in it is settled now, and silence never approves.
    pub decisions: Vec<ToolApprovalAnswer>,
}

/// How one pending request was settled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ToolApprovalResolution {
    /// The approval that was answered.
    pub tool_call_id: String,
    /// The gated tool.
    pub tool: String,
    /// `allow`, `allow_always`, `reject`, `reject_always`, `not_approved`
    /// (left out of the submission) or `expired`.
    #[cfg_attr(feature = "openapi", schema(example = "allow"))]
    pub outcome: String,
}

/// Result of answering tool-approval requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SubmitToolApprovalsResponse {
    /// How every pending request in the batch was settled.
    pub resolved: Vec<ToolApprovalResolution>,
    /// Session status after the decision.
    #[cfg_attr(feature = "openapi", schema(example = "active"))]
    pub status: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_kinds_serialize_as_tagged_objects() {
        let auth = vec![
            AgentCardAuth::AgentKey,
            AgentCardAuth::Oidc {
                issuer: "https://auth.example.com".into(),
            },
        ];
        assert_eq!(
            serde_json::to_value(&auth).unwrap(),
            serde_json::json!([
                {"type": "agent_key"},
                {"type": "oidc", "issuer": "https://auth.example.com"}
            ])
        );
    }

    #[test]
    fn approval_request_round_trips_the_server_shape() {
        let body = serde_json::json!({
            "decisions": [{"tool_call_id": "tool_approval_1", "decision": "allow_always"}]
        });
        let parsed: SubmitToolApprovalsRequest = serde_json::from_value(body.clone()).unwrap();
        assert!(parsed.decisions[0].decision.allows());
        assert_eq!(serde_json::to_value(&parsed).unwrap(), body);
    }

    #[test]
    fn card_omits_empty_optional_fields() {
        let card = AgentCard {
            name: "support".into(),
            description: None,
            streaming: true,
            input: AgentCardInput::TEXT,
            auth: vec![],
            conversation_starters: vec![],
            links: AgentCardLinks {
                sessions: "/v1/channels/support/sessions".into(),
                ag_ui: None,
                a2a: None,
            },
        };
        let json = serde_json::to_value(&card).unwrap();
        assert!(json.get("description").is_none());
        assert!(json.get("conversation_starters").is_none());
        assert!(json["links"].get("ag_ui").is_none());
    }
}
