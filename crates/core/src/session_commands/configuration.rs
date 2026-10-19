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
    /// Replace the session capability overlay: capabilities activated on this
    /// session in addition to its Agent and Harness layers (EVE-1297). Each
    /// entry names trusted, host-registered code by id; the configuration is
    /// validated by that code before a host commits the patch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Vec<everruns_contracts::CapabilityRef>>,
    /// Replace the live model selection; `Some(None)` returns to the Agent's
    /// model. A credential-free provider key and model id, never provider
    /// construction material.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "nullable_patch"
    )]
    pub model: Option<Option<everruns_contracts::model_spec::ModelSpec>>,
    /// Replace the live reasoning effort; `Some(None)` returns to effort
    /// carried by message controls.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "nullable_patch"
    )]
    pub reasoning_effort: Option<Option<everruns_contracts::ReasoningEffort>>,
}
fn instructions_patch<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}
fn nullable_patch<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(deserializer).map(Some)
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
        if self
            .capabilities
            .as_ref()
            .is_some_and(|capabilities| capabilities.len() > 64)
        {
            return Err(AgentLoopError::config(
                "session configuration accepts at most 64 session capabilities",
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
        if patch.capabilities.is_some() {
            self.capabilities = patch.capabilities;
        }
        if patch.model.is_some() {
            self.model = patch.model;
        }
        if patch.reasoning_effort.is_some() {
            self.reasoning_effort = patch.reasoning_effort;
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
        // The record's capabilities are its session layer; Agent and Harness
        // capabilities resolve from their own layers. Model and effort are
        // read by the Framework's runtime composition, not the record.
        if let Some(capabilities) = &self.capabilities {
            record.capabilities = capabilities.clone();
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

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::model_spec::ModelSpec;

    #[test]
    fn live_selections_merge_round_trip_and_clear_explicitly() {
        let mut retained = SessionConfiguration::default();
        retained.merge(SessionConfiguration {
            capabilities: Some(vec![everruns_contracts::CapabilityRef::new("overlay")]),
            model: Some(Some(ModelSpec::on("team", "large"))),
            reasoning_effort: Some(Some(everruns_contracts::ReasoningEffort::High)),
            ..Default::default()
        });
        // A patch that names nothing else leaves the retained selections.
        retained.merge(SessionConfiguration {
            instructions: Some(Some("x".into())),
            ..Default::default()
        });
        let decoded: SessionConfiguration =
            serde_json::from_value(serde_json::to_value(&retained).unwrap()).unwrap();
        assert_eq!(decoded.model, Some(Some(ModelSpec::on("team", "large"))));
        assert_eq!(
            decoded.reasoning_effort,
            Some(Some(everruns_contracts::ReasoningEffort::High))
        );
        assert_eq!(decoded.capabilities.as_ref().map(Vec::len), Some(1));

        // An explicit clear survives serialization as a clear, not as unset.
        retained.merge(SessionConfiguration {
            model: Some(None),
            ..Default::default()
        });
        let decoded: SessionConfiguration =
            serde_json::from_value(serde_json::to_value(&retained).unwrap()).unwrap();
        assert_eq!(decoded.model, Some(None));

        #[cfg(feature = "host")]
        {
            let mut record =
                crate::host::SessionBuilder::new(everruns_contracts::typed_id::HarnessId::new())
                    .capability(everruns_contracts::CapabilityRef::new("stale_copy"))
                    .build();
            decoded.apply(&mut record);
            assert_eq!(record.capabilities.len(), 1);
            assert_eq!(record.capabilities[0].capability_id(), "overlay");
        }
    }

    #[test]
    fn the_overlay_is_bounded() {
        let patch = SessionConfiguration {
            capabilities: Some(
                (0..65)
                    .map(|index| everruns_contracts::CapabilityRef::new(format!("c{index}")))
                    .collect(),
            ),
            ..Default::default()
        };
        assert!(patch.validate().is_err());
    }
}
