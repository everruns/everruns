use super::*;
/// A trusted host's patch to a session's model instructions and client tools.
/// These values never change executable Agent/Harness/Environment bindings.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SessionConfiguration {
    /// Replace run-scoped instructions, including an explicit empty value.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "instructions_patch"
    )]
    pub instructions: Option<Option<String>>,
    /// Replace the client-side tools, including an explicit empty set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_tools: Option<Vec<everruns_contracts::tool_types::ToolDefinition>>,
}
fn instructions_patch<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}
impl SessionConfiguration {
    /// Bound persisted client settings before an authority transaction.
    pub fn validate(&self) -> Result<()> {
        if serde_json::to_vec(self)
            .map_err(|e| AgentLoopError::config(e.to_string()))?
            .len()
            > 1024 * 1024
        {
            return Err(AgentLoopError::config(
                "session configuration exceeds 1 MiB",
            ));
        }
        if let Some(tools) = &self.client_tools
            && (tools.len() > 64
                || tools.iter().any(|tool| {
                    !matches!(
                        tool,
                        everruns_contracts::tool_types::ToolDefinition::ClientSide(_)
                    )
                }))
        {
            return Err(AgentLoopError::config(
                "session configuration accepts at most 64 client tools",
            ));
        }
        Ok(())
    }
    /// Merge a patch without resetting fields it did not supply.
    pub fn merge(&mut self, patch: Self) {
        if patch.instructions.is_some() {
            self.instructions = patch.instructions;
        }
        if patch.client_tools.is_some() {
            self.client_tools = patch.client_tools;
        }
    }
    /// Apply configuration to a read projection, never to executable definitions.
    pub fn apply(&self, record: &mut crate::ExecutionSession) {
        if let Some(instructions) = &self.instructions {
            record.system_prompt = instructions.clone();
        }
        if let Some(tools) = &self.client_tools {
            let hints = record.hints.get_or_insert_with(Default::default);
            if tools.is_empty() {
                hints.remove("setup_connection");
            } else {
                hints.insert("setup_connection".into(), true.into());
            }
            if hints.is_empty() {
                record.hints = None;
            }
            record.tools = tools.clone();
        }
    }
}

/// Validate a bounded import of prior conversation before an atomic empty-history seed.
pub fn validate_history_seed(messages: &[RuntimeMessage]) -> Result<()> {
    use crate::message::RuntimeMessageRole;
    if messages.len() > 4096
        || messages.iter().any(|message| {
            !matches!(
                message.role,
                RuntimeMessageRole::User | RuntimeMessageRole::Agent
            )
        })
        || serde_json::to_vec(messages)
            .map_err(|error| AgentLoopError::config(error.to_string()))?
            .len()
            > 4 * 1024 * 1024
    {
        return Err(AgentLoopError::config(
            "history seed must contain at most 4096 user/agent messages within 4 MiB",
        ));
    }
    Ok(())
}
