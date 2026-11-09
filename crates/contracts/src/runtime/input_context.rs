//! Who an admitted input acts for (data plane v2, "Execution context").
//!
//! Decision: some access depends on the person, not the agent: a person's
//! own MCP connections and OAuth tokens are exposed only when that person
//! wrote the input or takes part in the session. An agent definition can't
//! hold that, so the trusted host that admits an input (the gateway, after
//! verifying the org token) attaches this context, and the authority keeps
//! it with the input, in the `input.message` event's metadata under
//! [`INPUT_CONTEXT_METADATA_KEY`]. Event metadata is host-written: a client
//! can't set it through the message, whose own metadata is separate.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Event metadata key of the [`InputContext`] an input was admitted with.
pub const INPUT_CONTEXT_METADATA_KEY: &str = "input_context";

/// The verified identities an input acts for. Ids are the host's principal
/// ids, opaque here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputContext {
    /// The verified author of the input, when a person or service wrote it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// The session's participants when the input was admitted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub participants: Vec<String>,
    /// Resource grants the agent definition allows for them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grants: Vec<String>,
}

impl InputContext {
    /// The context stored in an `input.message` event's metadata, if any.
    pub fn from_event_metadata(metadata: Option<&Value>) -> Option<Self> {
        metadata?
            .get(INPUT_CONTEXT_METADATA_KEY)
            .and_then(|value| serde_json::from_value(value.clone()).ok())
    }
}
