use crate::engine::capabilities::CapabilityRegistry;
use crate::engine::event_emitter::EventEmitter;
use crate::engine::finalized_tool_calls::{FinalizedToolCallRejection, FinalizedToolCallsContext};
use crate::engine::tool_types::{ToolCall, ToolDefinition};
use crate::engine::typed_id::SessionId;
use crate::engine::{CapabilityRef, ExecutionContext};

#[allow(clippy::too_many_arguments)]
pub(super) async fn apply_finalized_tool_calls_hooks(
    capability_registry: &CapabilityRegistry,
    event_emitter: &dyn EventEmitter,
    session_id: SessionId,
    context: &ExecutionContext,
    resolved_capability_configs: &[CapabilityRef],
    tool_definitions: &[ToolDefinition],
    tool_calls: &mut [ToolCall],
    iteration: u32,
) -> Vec<FinalizedToolCallRejection> {
    let hook_context = FinalizedToolCallsContext {
        event_emitter,
        session_id,
        execution_context: context,
        tool_definitions,
        iteration,
    };
    let mut rejections = Vec::new();
    for config in resolved_capability_configs {
        let Some(capability) = capability_registry.get(config.capability_id()) else {
            continue;
        };
        if let Some(hook) = capability.finalized_tool_calls_hook(config.config_value()) {
            rejections.extend(hook.apply_with_rejections(&hook_context, tool_calls).await);
        }
    }
    rejections
}

/// Rejections precede Act, so contribute the same owned failure wording here.
pub(super) fn rejected_call_data(
    registry: &CapabilityRegistry,
    capabilities: &[CapabilityRef],
    definitions: &[ToolDefinition],
    call: &ToolCall,
    error: String,
    locale: Option<&str>,
) -> crate::engine::events::ToolCompletedData {
    use crate::engine::tool_narration::{
        ToolNarrationContext, ToolNarrationPhase, render_tool_narration_with_locale,
    };
    let definition = definitions
        .iter()
        .find(|definition| definition.name() == call.name);
    let phase = ToolNarrationPhase::Failed;
    let narration = capabilities
        .iter()
        .find_map(|config| {
            registry.get(config.capability_id()).and_then(|capability| {
                capability.narrate(
                    definition,
                    call,
                    phase,
                    locale,
                    ToolNarrationContext::default(),
                )
            })
        })
        .unwrap_or_else(|| render_tool_narration_with_locale(definition, call, phase, locale));
    crate::engine::events::ToolCompletedData::failure(
        call.id.clone(),
        call.name.clone(),
        "error".into(),
        error,
        None,
    )
    .with_display_name(
        definition
            .and_then(|definition| definition.display_name())
            .map(str::to_string),
    )
    .with_narration(Some(narration))
}

#[cfg(all(test, feature = "builtins"))]
mod tests {
    use super::*;
    use crate::builtins::ask_user::AskUserCapability;
    use crate::engine::capabilities::Capability;
    use serde_json::json;

    #[test]
    fn rejected_questions_have_owned_failure_narration() {
        let capability = AskUserCapability::client_side();
        let definitions = capability.tool_definitions();
        let mut registry = CapabilityRegistry::new();
        registry.register(capability);
        let call = ToolCall {
            id: "invalid-question".into(),
            name: "ask_user".into(),
            arguments: json!({"questions":[{"header":"Release scope","question":"PRIVATE_BODY","options":[]}]}),
        };
        for (locale, expected) in [
            (None, "Could not ask user: Release scope"),
            (
                Some("uk-UA"),
                "Не вдалося запитати користувача: Release scope",
            ),
        ] {
            let data = rejected_call_data(
                &registry,
                &[CapabilityRef::new("ask_user")],
                &definitions,
                &call,
                "invalid options".into(),
                locale,
            );
            assert_eq!(data.narration.as_deref(), Some(expected));
            assert!(!data.success);
        }
    }
}
