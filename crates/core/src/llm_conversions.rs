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
    use crate::message::{ImageFileContentPart, TextContentPart};

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
        let runtime_agent = RuntimeAgent::new("You are helpful", "gpt-4o");
        let llm_config = llm_call_config_builder_from_agent(&runtime_agent).build();

        assert_eq!(llm_config.model, "gpt-4o");
        assert!(llm_config.reasoning_effort.is_none());
        assert!(llm_config.temperature.is_none());
        assert!(llm_config.max_tokens.is_none());
        assert!(llm_config.tools.is_empty());
        assert!(llm_config.metadata.is_empty());
        // No server tools configured on the agent → none on the call config.
        assert!(llm_config.openrouter_routing.is_none());
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
    fn test_llm_call_config_builder_with_metadata() {
        let runtime_agent = RuntimeAgent::new("You are helpful", "gpt-4o");
        let llm_config = llm_call_config_builder_from_agent(&runtime_agent)
            .with_metadata("session_id", "session_abc123")
            .with_metadata("agent_id", "agent_xyz789")
            .build();

        assert_eq!(
            llm_config.metadata.get("session_id"),
            Some(&"session_abc123".to_string())
        );
        assert_eq!(
            llm_config.metadata.get("agent_id"),
            Some(&"agent_xyz789".to_string())
        );
    }

    #[test]
    fn test_llm_call_config_builder_with_metadata_hashmap() {
        let runtime_agent = RuntimeAgent::new("You are helpful", "gpt-4o");
        let mut metadata = HashMap::new();
        metadata.insert("key1".to_string(), "value1".to_string());
        metadata.insert("key2".to_string(), "value2".to_string());

        let llm_config = llm_call_config_builder_from_agent(&runtime_agent)
            .metadata(metadata)
            .build();

        assert_eq!(llm_config.metadata.get("key1"), Some(&"value1".to_string()));
        assert_eq!(llm_config.metadata.get("key2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_llm_call_config_builder_with_reasoning_effort() {
        let runtime_agent = RuntimeAgent::new("You are helpful", "gpt-4o");
        let llm_config = llm_call_config_builder_from_agent(&runtime_agent)
            .reasoning_effort("high")
            .build();

        assert_eq!(llm_config.reasoning_effort, Some("high".to_string()));
    }

    #[test]
    fn test_llm_call_config_builder_with_all_options() {
        let runtime_agent = RuntimeAgent::new("You are helpful", "gpt-4o");
        let llm_config = llm_call_config_builder_from_agent(&runtime_agent)
            .model("claude-3-opus")
            .reasoning_effort("medium")
            .temperature(0.7)
            .max_tokens(1000)
            .build();

        assert_eq!(llm_config.model, "claude-3-opus");
        assert_eq!(llm_config.reasoning_effort, Some("medium".to_string()));
        assert_eq!(llm_config.temperature, Some(0.7));
        assert_eq!(llm_config.max_tokens, Some(1000));
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
        let message = Message {
            id: uuid::Uuid::new_v4().into(),
            role: MessageRole::User,
            content: vec![
                ContentPart::Text(TextContentPart::new("Look at this image".to_string())),
                ContentPart::ImageFile(ImageFileContentPart {
                    image_id: uuid::Uuid::new_v4().into(),
                    filename: Some("test.png".to_string()),
                }),
            ],
            phase: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: chrono::Utc::now(),
        };

        assert!(message_has_image_files(&message));
    }

    #[test]
    fn test_message_has_image_files_without_image_file() {
        let message = Message {
            id: uuid::Uuid::new_v4().into(),
            role: MessageRole::User,
            content: vec![ContentPart::Text(TextContentPart::new(
                "Just text".to_string(),
            ))],
            phase: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: chrono::Utc::now(),
        };

        assert!(!message_has_image_files(&message));
    }

    #[test]
    fn test_extract_image_file_ids() {
        let id1 = uuid::Uuid::new_v4();
        let id2 = uuid::Uuid::new_v4();

        let message = Message {
            id: uuid::Uuid::new_v4().into(),
            role: MessageRole::User,
            content: vec![
                ContentPart::Text(TextContentPart::new("Look at these images".to_string())),
                ContentPart::ImageFile(ImageFileContentPart {
                    image_id: id1.into(),
                    filename: Some("test1.png".to_string()),
                }),
                ContentPart::ImageFile(ImageFileContentPart {
                    image_id: id2.into(),
                    filename: Some("test2.png".to_string()),
                }),
            ],
            phase: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: chrono::Utc::now(),
        };

        let ids = extract_image_file_ids(&message);
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
    }

    #[test]
    fn test_from_message_with_images_text_only() {
        let message = Message {
            id: uuid::Uuid::new_v4().into(),
            role: MessageRole::User,
            content: vec![ContentPart::Text(TextContentPart::new("Hello".to_string()))],
            phase: None,
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
        let image_id = uuid::Uuid::new_v4();
        let message = Message {
            id: uuid::Uuid::new_v4().into(),
            role: MessageRole::User,
            content: vec![
                ContentPart::Text(TextContentPart::new("Look at this".to_string())),
                ContentPart::ImageFile(ImageFileContentPart {
                    image_id: image_id.into(),
                    filename: Some("test.png".to_string()),
                }),
            ],
            phase: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: chrono::Utc::now(),
        };

        let mut resolved = std::collections::HashMap::new();
        resolved.insert(
            image_id,
            crate::image_services::ResolvedImage::new("base64data", "image/png"),
        );

        let llm_message = llm_message_from_message_with_images(&message, &resolved);

        match &llm_message.content {
            LlmMessageContent::Parts(parts) => {
                assert_eq!(parts.len(), 2);
                // First part should be text
                assert!(matches!(&parts[0], LlmContentPart::Text { .. }));
                // Second part should be resolved image
                if let LlmContentPart::Image { url } = &parts[1] {
                    assert!(url.starts_with("data:image/png;base64,"));
                } else {
                    panic!("Expected image content part");
                }
            }
            _ => panic!("Expected parts content"),
        }
    }

    #[test]
    fn test_from_message_with_images_unresolved_image() {
        let image_id = uuid::Uuid::new_v4();
        let message = Message {
            id: uuid::Uuid::new_v4().into(),
            role: MessageRole::User,
            content: vec![ContentPart::ImageFile(ImageFileContentPart {
                image_id: image_id.into(),
                filename: Some("missing.png".to_string()),
            })],
            phase: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: chrono::Utc::now(),
        };

        // Empty resolved map - image not found
        let resolved = std::collections::HashMap::new();
        let llm_message = llm_message_from_message_with_images(&message, &resolved);

        // Should have placeholder text for missing image
        // When there's only one part, it may return Text directly instead of Parts
        match &llm_message.content {
            LlmMessageContent::Text(text) => {
                assert!(text.contains("Image not found"));
            }
            LlmMessageContent::Parts(parts) => {
                assert_eq!(parts.len(), 1);
                if let LlmContentPart::Text { text } = &parts[0] {
                    assert!(text.contains("Image not found"));
                } else {
                    panic!("Expected text placeholder for missing image");
                }
            }
        }
    }
}
