//! Wire types for a tool call parked on a hard approval gate, as it crosses
//! from the `tool_approval` capability to the engine and the answer API.
//!
//! Spec: knowledge/execution/tool-approval.md. Re-exported from
//! [`crate::tool_types`], next to the elicitation equivalents: the builtins
//! crate produces the payload, the engine parks the turn on it, and the server
//! answers it, and none of the three depends on another.

use crate::tool_types::{ToolCall, ToolResult};
use serde::{Deserialize, Serialize};

/// `result.code` marking a tool result that stopped on a hard approval gate:
/// the call did not run, and a person has to approve it first.
pub const TOOL_APPROVAL_REQUIRED_CODE: &str = "tool_approval_required";

/// Name of the synthetic client-side call that carries an approval request to
/// a person. The engine emits it, the session UI renders a card for it, and
/// `POST /v1/sessions/{id}/tool-approvals` recognises it.
pub const APPROVE_TOOL_CALL_TOOL: &str = "approve_tool_call";

/// Prefix of the synthetic approval call's id. The rest is the gated call's own
/// id, so a replayed act derives the same request rather than a second one.
pub const TOOL_APPROVAL_CALL_ID_PREFIX: &str = "tool_approval_";

/// Budget, in serialized bytes, for a copy of a call's arguments that is
/// recorded for people to read: the approval card and the executed arguments
/// on `tool.completed`.
pub const TOOL_ARGUMENTS_PREVIEW_BYTES: usize = 8 * 1024;

/// Bounded copy of a call's arguments for a person to read.
///
/// Arguments within [`TOOL_ARGUMENTS_PREVIEW_BYTES`] come back unchanged. Larger
/// ones come back as their serialized JSON cut at a char boundary, as a string,
/// with `true` to say so.
pub fn preview_tool_arguments(arguments: &serde_json::Value) -> (serde_json::Value, bool) {
    let serialized = serde_json::to_string(arguments).unwrap_or_default();
    if serialized.len() <= TOOL_ARGUMENTS_PREVIEW_BYTES {
        return (arguments.clone(), false);
    }
    let mut end = TOOL_ARGUMENTS_PREVIEW_BYTES;
    while end > 0 && !serialized.is_char_boundary(end) {
        end -= 1;
    }
    (
        serde_json::Value::String(serialized[..end].to_string()),
        true,
    )
}

/// Structured payload of a tool result parked on a hard approval gate.
///
/// Everything a person needs to decide travels here, so the card and the answer
/// API read it back out of the engine-emitted request rather than trusting a
/// client to say what was approved.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolApprovalRequired {
    /// Always [`TOOL_APPROVAL_REQUIRED_CODE`]; discriminates the payload.
    pub code: String,
    /// Model-facing sentence explaining what is being waited on.
    pub error: String,
    /// Id of the gated call, as the model issued it.
    pub tool_call_id: String,
    /// Tool the model asked to run.
    pub tool: String,
    /// Human-readable tool name, when the definition has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// The call's arguments as a person should review them. Bounded: when the
    /// arguments are larger than the preview budget this holds a truncated
    /// JSON string and `arguments_truncated` is set. The approval still binds
    /// to the full arguments through `fingerprint`.
    pub arguments: serde_json::Value,
    /// True when `arguments` is a truncated preview.
    #[serde(default)]
    pub arguments_truncated: bool,
    /// Digest of the tool name and exact arguments. A one-off approval is
    /// recorded against it, so it only ever lets this exact call through.
    pub fingerprint: String,
    /// Why the gate asked: `destructive`, `open_world`, `mutating`, or `policy`.
    pub risk: String,
    /// Approval mode the gate ran under.
    pub mode: String,
    /// When the gate asked (RFC 3339).
    pub asked_at: String,
    /// When the server stops waiting and treats the request as rejected
    /// (RFC 3339).
    pub expires_at: String,
}

impl ToolApprovalRequired {
    /// Recover the payload from a tool result, if that is what it carries.
    pub fn from_tool_result(result: &ToolResult) -> Option<Self> {
        let value = result.result.as_ref()?;
        if value.get("code")?.as_str()? != TOOL_APPROVAL_REQUIRED_CODE {
            return None;
        }
        serde_json::from_value(value.clone()).ok()
    }

    /// The synthetic call that asks a person to decide.
    ///
    /// The id is derived from the gated call's id, so replaying the act that
    /// parked produces the same request.
    pub fn request_call(&self) -> ToolCall {
        ToolCall {
            id: format!("{TOOL_APPROVAL_CALL_ID_PREFIX}{}", self.tool_call_id),
            name: APPROVE_TOOL_CALL_TOOL.to_string(),
            arguments: serde_json::to_value(self).unwrap_or_default(),
        }
    }

    /// Recover the request from a synthetic `approve_tool_call` call.
    pub fn from_request_call(id: &str, name: &str, arguments: &serde_json::Value) -> Option<Self> {
        if name != APPROVE_TOOL_CALL_TOOL || !id.starts_with(TOOL_APPROVAL_CALL_ID_PREFIX) {
            return None;
        }
        if arguments.get("code")?.as_str()? != TOOL_APPROVAL_REQUIRED_CODE {
            return None;
        }
        serde_json::from_value(arguments.clone()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload() -> serde_json::Value {
        json!({"code":"tool_approval_required","error":"Waiting","tool_call_id":"call_1",
            "tool":"send_email","arguments":{"to":"a@example.com"},"fingerprint":"sha256:ab",
            "risk":"open_world","mode":"normal","asked_at":"2026-10-01T00:00:00Z",
            "expires_at":"2026-10-01T00:15:00Z"})
    }

    #[test]
    fn approval_payload_requires_its_discriminator() {
        let result = ToolResult {
            tool_call_id: "call_1".into(),
            result: Some(payload()),
            images: None,
            error: Some("Waiting".into()),
            connection_required: None,
            raw_output: None,
        };
        let parsed = ToolApprovalRequired::from_tool_result(&result).expect("parsed");
        assert_eq!(parsed.tool, "send_email");
        assert!(!parsed.arguments_truncated);

        let other = ToolResult {
            result: Some(json!({"code":"url_elicitation_required"})),
            ..result
        };
        assert!(ToolApprovalRequired::from_tool_result(&other).is_none());
    }

    #[test]
    fn request_call_round_trips_and_is_derived_from_the_gated_call() {
        let request: ToolApprovalRequired = serde_json::from_value(payload()).unwrap();
        let call = request.request_call();
        assert_eq!(call.id, "tool_approval_call_1");
        assert_eq!(call.name, APPROVE_TOOL_CALL_TOOL);
        assert_eq!(
            ToolApprovalRequired::from_request_call(&call.id, &call.name, &call.arguments),
            Some(request.clone())
        );
        // A model-authored call with the same name but not the engine's id
        // prefix is not an approval request.
        assert!(
            ToolApprovalRequired::from_request_call("call_9", &call.name, &call.arguments)
                .is_none()
        );
        assert!(
            ToolApprovalRequired::from_request_call(&call.id, "ask_user", &call.arguments)
                .is_none()
        );
    }

    #[test]
    fn large_arguments_are_previewed_not_copied() {
        let big = json!({ "body": "é".repeat(20_000) });
        let (preview, truncated) = preview_tool_arguments(&big);
        assert!(truncated);
        assert!(preview.as_str().unwrap().len() <= TOOL_ARGUMENTS_PREVIEW_BYTES);
        let small = json!({ "body": "x" });
        assert_eq!(preview_tool_arguments(&small), (small, false));
    }
}
