//! Conversions between core agent-loop domain types and the provider driver
//! types in `everruns-provider`.
//!
//! These adapters live here (not in `everruns-provider`) because they depend on
//! core domain types (`Message`, `RuntimeAgent`). Keeping them
//! on the core side keeps the crate dependency one-directional: core depends on
//! everruns-provider, never the reverse. The orphan rule also prevents these
//! from being `From` impls in core (both the `From` trait and the driver types
//! are foreign to core), so they are plain functions.

use std::collections::HashMap;
use uuid::Uuid;

use crate::driver_registry::{
    LlmCallConfig, LlmCallConfigBuilder, LlmContentPart, LlmMessage, LlmMessageContent,
    LlmMessageRole, truncate_tool_result,
};
use crate::image_services::ResolvedImage;
use crate::message::{ContentPart, Message, MessageRole};
use crate::runtime_agent::RuntimeAgent;
use crate::tool_types::ToolCall;

/// Convert a [`Message`] into an [`LlmMessage`] (text-only; images become
/// placeholders). For multimodal messages use
/// [`llm_message_from_message_with_images`].
pub fn llm_message_from_message(msg: &Message) -> LlmMessage {
    let role = match msg.role {
        MessageRole::System => LlmMessageRole::System,
        MessageRole::User => LlmMessageRole::User,
        MessageRole::Agent => LlmMessageRole::Assistant,
        MessageRole::ToolResult => LlmMessageRole::Tool,
    };

    let tool_calls: Vec<ToolCall> = msg
        .tool_calls()
        .into_iter()
        .map(|tc| ToolCall {
            id: tc.id.clone(),
            name: tc.name.clone(),
            arguments: tc.arguments.clone(),
        })
        .collect();

    LlmMessage {
        configuration_update: None,
        role,
        content: LlmMessageContent::Text(msg.content_to_llm_string()),
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
        tool_call_id: msg.tool_call_id().map(|s| s.to_string()),
        phase: msg.phase,
        reasoning: msg.reasoning_parts().cloned().collect(),
    }
}

/// Convert a [`Message`] into an [`LlmMessage`] with resolved images.
///
/// - text parts -> `LlmContentPart::Text`
/// - inline image parts -> `LlmContentPart::Image` (data URL)
/// - image_file parts -> resolved to a data URL, or a placeholder if missing
/// - tool_call parts -> extracted to the `tool_calls` field
/// - tool_result parts -> text representation (truncated by the same backstop
///   the tool scheduler applies)
pub fn llm_message_from_message_with_images(
    msg: &Message,
    resolved_images: &HashMap<Uuid, ResolvedImage>,
) -> LlmMessage {
    let role = match msg.role {
        MessageRole::System => LlmMessageRole::System,
        MessageRole::User => LlmMessageRole::User,
        MessageRole::Agent => LlmMessageRole::Assistant,
        MessageRole::ToolResult => LlmMessageRole::Tool,
    };

    let mut parts: Vec<LlmContentPart> = Vec::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    for part in &msg.content {
        match part {
            // Reasoning travels on `LlmMessage::reasoning`, not in the content
            // parts: drivers replay it in provider-native form and it must
            // never be flattened into prompt text.
            ContentPart::Reasoning(_) => {}
            ContentPart::Text(t) => {
                parts.push(LlmContentPart::Text {
                    text: t.text.clone(),
                });
            }
            ContentPart::Image(img) => {
                if let Some(url) = &img.url {
                    parts.push(LlmContentPart::Image { url: url.clone() });
                } else if let (Some(base64), Some(media_type)) = (&img.base64, &img.media_type) {
                    let data_url = format!("data:{};base64,{}", media_type, base64);
                    parts.push(LlmContentPart::Image { url: data_url });
                }
            }
            ContentPart::ImageFile(img_file) => {
                if let Some(resolved) = resolved_images.get(&img_file.image_id.uuid()) {
                    parts.push(LlmContentPart::Image {
                        url: resolved.to_data_url(),
                    });
                } else {
                    parts.push(LlmContentPart::Text {
                        text: format!("[Image not found: {}]", img_file.image_id),
                    });
                }
            }
            ContentPart::ToolCall(tc) => {
                tool_calls.push(ToolCall {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                });
            }
            ContentPart::ToolResult(tr) => {
                let text = if let Some(err) = &tr.error {
                    format!("Tool error: {}", err)
                } else if let Some(res) = &tr.result {
                    serde_json::to_string(res).unwrap_or_else(|_| "{}".to_string())
                } else {
                    "{}".to_string()
                };
                let text = truncate_tool_result(text);
                parts.push(LlmContentPart::Text { text });
            }
        }
    }

    let content = if parts.len() == 1 && matches!(&parts[0], LlmContentPart::Text { .. }) {
        if let LlmContentPart::Text { text } = &parts[0] {
            LlmMessageContent::Text(text.clone())
        } else {
            LlmMessageContent::Parts(parts)
        }
    } else if parts.is_empty() {
        LlmMessageContent::Text(String::new())
    } else {
        LlmMessageContent::Parts(parts)
    };

    LlmMessage {
        configuration_update: None,
        role,
        content,
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
        tool_call_id: msg.tool_call_id().map(|s| s.to_string()),
        phase: msg.phase,
        reasoning: msg.reasoning_parts().cloned().collect(),
    }
}

/// Whether a message contains image_file references that need resolution.
pub fn message_has_image_files(msg: &Message) -> bool {
    msg.content.iter().any(|p| p.is_image_file())
}

/// Extract all image_file IDs from a message.
pub fn extract_image_file_ids(msg: &Message) -> Vec<Uuid> {
    msg.content
        .iter()
        .filter_map(|p| match p {
            ContentPart::ImageFile(f) => Some(f.image_id.uuid()),
            _ => None,
        })
        .collect()
}

/// Seed an [`LlmCallConfig`] from a [`RuntimeAgent`]. Fields set later by the
/// ReasonAtom (reasoning effort, speed, verbosity, metadata) are left unset.
pub fn llm_call_config_from_agent(runtime_agent: &RuntimeAgent) -> LlmCallConfig {
    LlmCallConfig {
        model: runtime_agent.model.clone(),
        temperature: runtime_agent.temperature,
        max_tokens: runtime_agent.max_tokens,
        tools: runtime_agent.tools.clone(),
        reasoning_effort: None,
        speed: None,
        verbosity: None,
        metadata: HashMap::new(),
        previous_response_id: None,
        provider_opaque_context: None,
        tool_search: runtime_agent.tool_search.clone(),
        prompt_cache: runtime_agent.prompt_cache.clone(),
        openrouter_routing: runtime_agent.openrouter_routing.clone(),
        parallel_tool_calls: runtime_agent.parallel_tool_calls,
        volatile_suffix_len: 0,
        extra_headers: Vec::new(),
        cache_diagnostics: None,
        reasoning_state: None,
    }
}

/// Start an [`LlmCallConfigBuilder`] from a [`RuntimeAgent`].
pub fn llm_call_config_builder_from_agent(runtime_agent: &RuntimeAgent) -> LlmCallConfigBuilder {
    LlmCallConfigBuilder::from_config(llm_call_config_from_agent(runtime_agent))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver_registry::{
        LlmContentPart, LlmMessageContent, LlmMessageRole, OpenRouterRoutingConfig,
        OpenRouterServerTool, OpenRouterServerToolKind,
    };
    use crate::message::TextContentPart;
    use everruns_provider::model::ReasoningEffort;

    #[test]
    fn test_resolved_parallel_tool_calls_gating() {
        let mut config = llm_call_config_from_agent(&RuntimeAgent::new("p", "gpt-5.2"));

        // No preference => always None.
        assert_eq!(config.resolved_parallel_tool_calls(true), None);
        assert_eq!(config.resolved_parallel_tool_calls(false), None);

        // Preference passes through only when the driver/model supports it.
        config.parallel_tool_calls = Some(true);
        assert_eq!(config.resolved_parallel_tool_calls(true), Some(true));
        assert_eq!(config.resolved_parallel_tool_calls(false), None);

        config.parallel_tool_calls = Some(false);
        assert_eq!(config.resolved_parallel_tool_calls(true), Some(false));
        assert_eq!(config.resolved_parallel_tool_calls(false), None);
    }

    #[test]
    fn test_llm_call_config_builder_from_runtime_agent() {
        let runtime_agent = RuntimeAgent::new("You are helpful", "gpt-5.2");
        let llm_config = llm_call_config_builder_from_agent(&runtime_agent).build();

        assert_eq!(llm_config.model, "gpt-5.2");
        assert!(llm_config.reasoning_effort.is_none());
        assert!(llm_config.temperature.is_none());
        assert!(llm_config.max_tokens.is_none());
        assert!(llm_config.tools.is_empty());
        assert!(llm_config.metadata.is_empty());
        // No server tools configured on the agent → none on the call config.
        assert!(llm_config.openrouter_routing.is_none());
        let mut populated = RuntimeAgent::new("prompt", "custom-model");
        populated.temperature = Some(0.25);
        populated.max_tokens = Some(321);
        populated.parallel_tool_calls = Some(false);
        populated.tools = vec![serde_json::from_value(serde_json::json!({
            "type":"builtin", "name":"lookup", "description":"Look up", "parameters":{"type":"object"}
        })).unwrap()];
        populated.tool_search = Some(crate::driver_registry::ToolSearchConfig {
            enabled: true,
            threshold: 17,
        });
        populated.prompt_cache = Some(crate::driver_registry::PromptCacheConfig {
            enabled: true,
            strategy: crate::driver_registry::PromptCacheStrategy::Auto,
            gemini_cached_content: Some("cachedContents/test".into()),
        });
        for actual in [
            llm_call_config_from_agent(&populated),
            llm_call_config_builder_from_agent(&populated).build(),
        ] {
            assert_eq!(actual.model, "custom-model");
            assert_eq!(actual.temperature, Some(0.25));
            assert_eq!(actual.max_tokens, Some(321));
            assert_eq!(actual.parallel_tool_calls, Some(false));
            assert_eq!(
                serde_json::to_value(actual.tools).unwrap(),
                serde_json::to_value(&populated.tools).unwrap()
            );
            assert_eq!(
                serde_json::to_value(actual.tool_search).unwrap(),
                serde_json::json!({"enabled":true,"threshold":17})
            );
            assert_eq!(actual.prompt_cache, populated.prompt_cache);
            assert!(actual.reasoning_effort.is_none());
            assert!(actual.metadata.is_empty());
        }
    }

    #[test]
    fn runtime_agent_openrouter_routing_flows_into_call_config() {
        // Closes the assembly loop: a capability sets RuntimeAgent.openrouter_routing
        // (server tools), and the From<&RuntimeAgent> conversion the reason atom
        // uses must carry it through to the OpenRouter driver.
        let mut runtime_agent = RuntimeAgent::new("You are helpful", "openai/gpt-5-mini");
        runtime_agent.openrouter_routing = Some(OpenRouterRoutingConfig {
            server_tools: vec![OpenRouterServerTool::new(
                OpenRouterServerToolKind::WebSearch,
            )],
            ..Default::default()
        });

        let llm_config = llm_call_config_from_agent(&runtime_agent);
        let routing = llm_config
            .openrouter_routing
            .expect("server-tool routing survives into the call config");
        assert_eq!(routing.server_tools.len(), 1);
        assert_eq!(
            routing.server_tools[0].kind.wire_type(),
            "openrouter:web_search"
        );
    }

    #[test]
    fn test_llm_call_config_builder_with_all_options() {
        let mut agent = RuntimeAgent::new("prompt", "original-model");
        agent.parallel_tool_calls = Some(false);
        agent.max_tokens = Some(12);
        let config = llm_call_config_builder_from_agent(&agent)
            .model("override-model")
            .reasoning_effort(ReasoningEffort::Medium)
            .temperature(0.5)
            .max_tokens(1000)
            .metadata(HashMap::from([
                ("session_id".into(), "old".into()),
                ("agent_id".into(), "agent_2".into()),
            ]))
            .with_metadata("session_id", "session_3")
            .with_metadata("trace", "trace_4")
            .build();
        assert_eq!(config.model, "override-model");
        assert_eq!(config.reasoning_effort, Some(ReasoningEffort::Medium));
        assert_eq!(config.temperature, Some(0.5));
        assert_eq!(config.max_tokens, Some(1000));
        assert_eq!(config.parallel_tool_calls, Some(false));
        assert_eq!(
            config.metadata,
            HashMap::from([
                ("session_id".into(), "session_3".into()),
                ("agent_id".into(), "agent_2".into()),
                ("trace".into(), "trace_4".into())
            ])
        );
    }

    #[test]
    fn test_llm_call_config_builder_with_openrouter_routing() {
        let runtime_agent = RuntimeAgent::new("You are helpful", "openai/gpt-5-mini");
        let routing = OpenRouterRoutingConfig::fallback_models([
            "openai/gpt-5-mini",
            "anthropic/claude-sonnet-4.5",
        ]);

        let llm_config = llm_call_config_builder_from_agent(&runtime_agent)
            .openrouter_routing(routing.clone())
            .build();

        assert_eq!(llm_config.openrouter_routing, Some(routing));
    }

    #[test]
    fn test_message_has_image_files_with_image_file() {
        let mut message = Message::user("Just text");
        assert!(!message_has_image_files(&message));
        message
            .content
            .push(ContentPart::image_url("https://example.com/image"));
        assert!(!message_has_image_files(&message));
        message
            .content
            .push(ContentPart::image_file(crate::typed_id::ImageId::new()));
        assert!(message_has_image_files(&message));
    }

    #[test]
    fn test_extract_image_file_ids() {
        let first = crate::typed_id::ImageId::new();
        let second = crate::typed_id::ImageId::new();
        let mut message = Message::user("images");
        message.content.extend([
            ContentPart::image_file(first),
            ContentPart::image_url("https://example.com/inline"),
            ContentPart::image_file(second),
            ContentPart::image_file(first),
        ]);
        assert_eq!(
            extract_image_file_ids(&message),
            vec![first.uuid(), second.uuid(), first.uuid()]
        );
        assert!(extract_image_file_ids(&Message::user("text")).is_empty());
    }

    #[test]
    fn test_from_message_with_images_text_only() {
        let message = Message {
            id: uuid::Uuid::new_v4().into(),
            role: MessageRole::User,
            content: vec![ContentPart::Text(TextContentPart::new("Hello".to_string()))],
            phase: None,
            phase_source: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: chrono::Utc::now(),
        };

        let resolved = std::collections::HashMap::new();
        let llm_message = llm_message_from_message_with_images(&message, &resolved);

        assert_eq!(llm_message.role, LlmMessageRole::User);
        match llm_message.content {
            LlmMessageContent::Text(text) => assert_eq!(text, "Hello"),
            _ => panic!("Expected text content"),
        }
    }

    #[test]
    fn test_from_message_with_images_resolved_image() {
        let first = crate::typed_id::ImageId::new();
        let second = crate::typed_id::ImageId::new();
        let mut message = Message::user("Look at this");
        message.content.extend([
            ContentPart::image_file(first),
            ContentPart::text("and this"),
            ContentPart::image_file(second),
        ]);
        let resolved = HashMap::from([
            (
                second.uuid(),
                ResolvedImage::new("second-data", "image/jpeg"),
            ),
            (
                Uuid::new_v4(),
                ResolvedImage::new("unused-data", "image/png"),
            ),
            (first.uuid(), ResolvedImage::new("first-data", "image/png")),
        ]);
        let actual = llm_message_from_message_with_images(&message, &resolved);
        assert_eq!(actual.role, LlmMessageRole::User);
        assert!(
            matches!(&actual.content, LlmMessageContent::Parts(parts) if matches!(&parts[..],
            [LlmContentPart::Text { text: a }, LlmContentPart::Image { url: b },
             LlmContentPart::Text { text: c }, LlmContentPart::Image { url: d }]
            if a == "Look at this" && b == "data:image/png;base64,first-data" && c == "and this" && d == "data:image/jpeg;base64,second-data"))
        );
    }

    #[test]
    fn test_from_message_with_images_unresolved_image() {
        let image_id = crate::typed_id::ImageId::from_uuid(Uuid::from_u128(7));
        let mut message = Message::user("");
        message.content = vec![ContentPart::image_file(image_id)];
        let actual = llm_message_from_message_with_images(&message, &HashMap::new());
        assert_eq!(actual.role, LlmMessageRole::User);
        let LlmMessageContent::Text(text) = actual.content else {
            panic!("single missing-image placeholder must remain text");
        };
        assert_eq!(
            text,
            "[Image not found: img_00000000000000000000000000000007]"
        );
    }

    #[test]
    fn adapters_preserve_native_reasoning_without_flattening_replay_secrets() {
        use everruns_provider::execution_phase::ExecutionPhase;
        use everruns_provider::reasoning::ReasoningContentPart;
        let calls = vec![ToolCall {
            id: "call_1".into(),
            name: "lookup".into(),
            arguments: serde_json::json!({"q":"x"}),
        }];
        let reasoning = ReasoningContentPart::opaque("test")
            .with_item_id("rs_1")
            .with_signature("private-signature")
            .with_encrypted("private-encrypted");
        let mut message = Message::assistant_with_tools("answer", calls.clone())
            .with_phase(ExecutionPhase::Commentary);
        message
            .content
            .push(ContentPart::reasoning(reasoning.clone()));
        for (actual, expected_text) in [
            (
                llm_message_from_message(&message),
                "answer\nTool call: lookup with arguments: {\"q\":\"x\"}",
            ),
            (
                llm_message_from_message_with_images(&message, &HashMap::new()),
                "answer",
            ),
        ] {
            assert_eq!(actual.role, LlmMessageRole::Assistant);
            assert_eq!(actual.phase, Some(ExecutionPhase::Commentary));
            assert_eq!(actual.content.to_text(), expected_text);
            assert_eq!(
                serde_json::to_value(actual.tool_calls).unwrap(),
                serde_json::to_value(&calls).unwrap()
            );
            assert_eq!(
                serde_json::to_value(actual.reasoning).unwrap(),
                serde_json::to_value(vec![reasoning.clone()]).unwrap()
            );
        }
    }
}
