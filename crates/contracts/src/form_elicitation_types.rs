//! Wire types for an MCP server's form mode elicitation, as it crosses from the
//! MCP executor to the engine and the answer API.
//!
//! Re-exported from
//! [`crate::tool_types`], next to the URL mode equivalents.

use crate::tool_types::ToolResult;
use serde::{Deserialize, Serialize};

/// `result.code` marking a tool result that stopped on a form mode elicitation:
/// an attached MCP server asked structured questions a person has to answer.
///
/// The MCP executor produces it; the engine's `FormElicitationHook` consumes it
/// and parks the turn on an `ask_user` card.
pub const FORM_ELICITATION_REQUIRED_CODE: &str = "form_elicitation_required";

/// Argument key that marks an `ask_user` call as engine-authored on behalf of an
/// MCP server's form elicitation, and carries who asked.
///
/// A model cannot set it: `ask_user` normalization rejects unknown fields. The
/// API that collects the answer additionally requires the engine's call id
/// prefix ([`FORM_ELICITATION_CALL_ID_PREFIX`]).
pub const MCP_ELICITATION_ARGUMENT: &str = "mcp_elicitation";

/// Prefix of the call id the engine gives a synthetic form elicitation
/// `ask_user` call. Model call ids come from the provider and never carry it.
pub const FORM_ELICITATION_CALL_ID_PREFIX: &str = "mcp_form_elicitation_";

/// Structured payload of a tool result that stopped on a form mode
/// elicitation.
///
/// The questions were projected from the server's `requestedSchema` and already
/// carry Everruns-composed attribution; credential-shaped and out-of-profile
/// schemas never get this far.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FormElicitationRequired {
    /// Always [`FORM_ELICITATION_REQUIRED_CODE`]; discriminates the payload.
    pub code: String,
    /// Model-facing sentence explaining what is being waited on.
    pub error: String,
    /// Logical MCP server that asked.
    pub server: String,
    /// MCP tool the elicitation interrupted. The answer is recorded against
    /// this pair.
    pub tool: String,
    /// The tool as the model knows it (`mcp_<server>_<tool>`).
    pub retry_tool: String,
    /// The server's own explanation of why it is asking. Server-authored.
    pub message: String,
    /// `ask_user` questions, one per schema property.
    pub questions: Vec<serde_json::Value>,
    /// Digest of the schema the questions came from, so an answer is only ever
    /// sent back for the questions the person saw.
    pub fingerprint: String,
}

impl FormElicitationRequired {
    /// Recover the payload from a tool result, if that is what it carries.
    pub fn from_tool_result(result: &ToolResult) -> Option<Self> {
        let value = result.result.as_ref()?;
        if value.get("code")?.as_str()? != FORM_ELICITATION_REQUIRED_CODE {
            return None;
        }
        serde_json::from_value(value.clone()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn form_elicitation_payload_requires_its_discriminator() {
        let payload = json!({"code":"form_elicitation_required","error":"Waiting",
            "server":"billing","tool":"buy","retry_tool":"mcp_billing_buy","message":"Pick",
            "questions":[{"id":"plan"}],"fingerprint":"abc"});
        let result = ToolResult {
            tool_call_id: "call".into(),
            result: Some(payload),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        };
        let parsed = FormElicitationRequired::from_tool_result(&result).expect("parsed");
        assert_eq!(parsed.retry_tool, "mcp_billing_buy");
        let other = ToolResult {
            result: Some(json!({"code":"url_elicitation_required"})),
            ..result
        };
        assert!(FormElicitationRequired::from_tool_result(&other).is_none());
    }
}
