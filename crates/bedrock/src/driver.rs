// AWS Bedrock Runtime LLM Driver
//
// Implements LlmDriver using the AWS Bedrock Runtime ConverseStream API.
// Uses the aws-sdk-bedrockruntime crate for typed event stream handling,
// which handles SigV4 signing and binary event stream framing internally.
//
// Credential shape: api_key is a JSON object (see credential.rs).
// base_url is unused.

use async_trait::async_trait;
use aws_sdk_bedrockruntime::Client;
use aws_sdk_bedrockruntime::config::{
    BehaviorVersion, Builder as BedrockConfigBuilder, Credentials, Region,
};
use aws_sdk_bedrockruntime::types::{
    ContentBlock, ContentBlockDelta, ContentBlockStart, ConversationRole, ConverseStreamOutput,
    ImageBlock, ImageFormat, ImageSource, InferenceConfiguration, Message, SystemContentBlock,
    Tool, ToolConfiguration, ToolInputSchema, ToolResultBlock, ToolResultContentBlock,
    ToolSpecification, ToolUseBlock,
};
use aws_smithy_types::Document;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::llm_driver_registry::{
    BoxedLlmDriver, DiscoveredModel, DriverRegistry, LlmCallConfig, LlmCompletionMetadata,
    LlmContentPart, LlmDriver, LlmMessage, LlmMessageContent, LlmMessageRole, LlmResponseStream,
    LlmStreamEvent, ProviderType,
};
use everruns_core::tool_types::{ToolCall, ToolDefinition};
use serde_json::Value;
use std::collections::HashMap;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::warn;

use crate::credential::BedrockCredential;

/// AWS Bedrock Runtime LLM Driver.
///
/// Implements `LlmDriver` using the `ConverseStream` API.
/// Credentials are parsed from the JSON-encoded `api_key` field.
#[derive(Clone, Debug)]
pub struct BedrockLlmDriver {
    credential: BedrockCredential,
}

impl BedrockLlmDriver {
    pub fn new(api_key: &str) -> Result<Self> {
        let credential = BedrockCredential::from_api_key(api_key)?;
        Ok(Self { credential })
    }

    fn create_client(&self) -> Client {
        let creds = Credentials::new(
            self.credential.access_key_id.clone(),
            self.credential.secret_access_key.clone(),
            self.credential.session_token.clone(),
            None,
            "everruns-bedrock",
        );
        let config = BedrockConfigBuilder::new()
            .behavior_version(BehaviorVersion::latest())
            .credentials_provider(creds)
            .region(Region::new(self.credential.region.clone()))
            .build();
        Client::from_conf(config)
    }
}

/// Register the Bedrock driver with the given registry.
pub fn register_driver(registry: &mut DriverRegistry) {
    registry.register(
        ProviderType::Bedrock,
        |api_key, _base_url| match BedrockLlmDriver::new(api_key) {
            Ok(driver) => Box::new(driver) as BoxedLlmDriver,
            Err(e) => Box::new(FailDriver(e.to_string())) as BoxedLlmDriver,
        },
    );
}

/// Driver that immediately fails with a credential error.
struct FailDriver(String);

#[async_trait]
impl LlmDriver for FailDriver {
    async fn chat_completion_stream(
        &self,
        _messages: Vec<LlmMessage>,
        _config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        Err(AgentLoopError::llm(self.0.clone()))
    }
}

#[async_trait]
impl LlmDriver for BedrockLlmDriver {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        let client = self.create_client();
        let model_id = config.model.clone();

        let (system_blocks, bedrock_messages) = build_messages(&messages)?;

        let tool_cfg = if !config.tools.is_empty() {
            Some(build_tool_config(&config.tools)?)
        } else {
            None
        };

        let inference_cfg = InferenceConfiguration::builder()
            .set_temperature(config.temperature.map(|t| t as f32))
            .set_max_tokens(config.max_tokens.map(|t| t as i32))
            .build();

        let mut req = client.converse_stream().model_id(&model_id);

        for msg in bedrock_messages {
            req = req.messages(msg);
        }

        if !system_blocks.is_empty() {
            req = req.set_system(Some(system_blocks));
        }

        if let Some(tc) = tool_cfg {
            req = req.tool_config(tc);
        }

        req = req.inference_config(inference_cfg);

        let response = req.send().await.map_err(|e| {
            let msg = format!("{e}");
            if is_too_large(&msg) {
                AgentLoopError::request_too_large(msg)
            } else {
                AgentLoopError::llm(format!("Bedrock ConverseStream failed: {e}"))
            }
        })?;

        let mut event_stream = response.stream;
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<LlmStreamEvent>>();

        tokio::spawn(async move {
            let mut pending: HashMap<usize, PartialToolCall> = HashMap::new();
            let mut meta = LlmCompletionMetadata::default();

            loop {
                match event_stream.recv().await {
                    Ok(Some(event)) => match event {
                        ConverseStreamOutput::ContentBlockDelta(e) => {
                            let idx = e.content_block_index() as usize;
                            match e.delta() {
                                Some(ContentBlockDelta::Text(t)) => {
                                    let _ = tx.send(Ok(LlmStreamEvent::TextDelta(t.clone())));
                                }
                                Some(ContentBlockDelta::ToolUse(t)) => {
                                    if let Some(tc) = pending.get_mut(&idx) {
                                        tc.input_json.push_str(t.input());
                                    }
                                }
                                _ => {}
                            }
                        }
                        ConverseStreamOutput::ContentBlockStart(e) => {
                            let idx = e.content_block_index() as usize;
                            if let Some(ContentBlockStart::ToolUse(tu)) = e.start() {
                                pending.insert(
                                    idx,
                                    PartialToolCall {
                                        id: tu.tool_use_id().to_string(),
                                        name: tu.name().to_string(),
                                        input_json: String::new(),
                                    },
                                );
                            }
                        }
                        ConverseStreamOutput::MessageStop(e) => {
                            meta.finish_reason = Some(e.stop_reason().as_str().to_string());
                            // Emit accumulated tool calls now; Done is emitted on stream end.
                            if !pending.is_empty() {
                                let mut ordered: Vec<(usize, PartialToolCall)> =
                                    pending.drain().collect();
                                ordered.sort_by_key(|(idx, _)| *idx);
                                let calls: Vec<ToolCall> = ordered
                                    .into_iter()
                                    .map(|(_, ptc)| ToolCall {
                                        id: ptc.id,
                                        name: ptc.name,
                                        arguments: serde_json::from_str(&ptc.input_json)
                                            .unwrap_or(Value::Object(serde_json::Map::new())),
                                    })
                                    .collect();
                                let _ = tx.send(Ok(LlmStreamEvent::ToolCalls(calls)));
                            }
                        }
                        ConverseStreamOutput::Metadata(e) => {
                            // Metadata arrives after MessageStop and carries token usage.
                            if let Some(usage) = e.usage() {
                                let prompt = usage.input_tokens() as u32;
                                let completion = usage.output_tokens() as u32;
                                meta.prompt_tokens = Some(prompt);
                                meta.completion_tokens = Some(completion);
                                meta.total_tokens = Some(prompt + completion);
                            }
                        }
                        _ => {} // MessageStart, ContentBlockStop, Unknown — no action needed
                    },
                    Ok(None) => {
                        // Stream ended — emit Done with accumulated metadata.
                        let _ = tx.send(Ok(LlmStreamEvent::Done(Box::new(meta))));
                        return;
                    }
                    Err(e) => {
                        let msg = format!("{e}");
                        let err = if is_too_large(&msg) {
                            AgentLoopError::request_too_large(msg)
                        } else {
                            AgentLoopError::llm(format!("Bedrock stream error: {e}"))
                        };
                        let _ = tx.send(Err(err));
                        return;
                    }
                }
            }
        });

        Ok(Box::pin(UnboundedReceiverStream::new(rx)))
    }

    async fn list_models(&self) -> Result<Option<Vec<DiscoveredModel>>> {
        // Converse-compatible model set is seeded statically; skip discovery.
        Ok(None)
    }
}

// ============================================================================
// Message conversion
// ============================================================================

struct PartialToolCall {
    id: String,
    name: String,
    input_json: String,
}

fn build_messages(messages: &[LlmMessage]) -> Result<(Vec<SystemContentBlock>, Vec<Message>)> {
    let mut system_blocks: Vec<SystemContentBlock> = Vec::new();
    let mut bedrock_messages: Vec<Message> = Vec::new();

    let mut i = 0;
    while i < messages.len() {
        let msg = &messages[i];
        match msg.role {
            LlmMessageRole::System => {
                match &msg.content {
                    LlmMessageContent::Text(text) if !text.is_empty() => {
                        system_blocks.push(SystemContentBlock::Text(text.clone()));
                    }
                    LlmMessageContent::Parts(parts) => {
                        for part in parts {
                            if let LlmContentPart::Text { text } = part {
                                if !text.is_empty() {
                                    system_blocks.push(SystemContentBlock::Text(text.clone()));
                                }
                            }
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            LlmMessageRole::Tool => {
                // Collect consecutive Tool messages into one User message.
                let mut tool_blocks: Vec<ContentBlock> = Vec::new();
                while i < messages.len() && messages[i].role == LlmMessageRole::Tool {
                    let tm = &messages[i];
                    if let Some(block) = build_tool_result_block(tm) {
                        tool_blocks.push(block);
                    }
                    i += 1;
                }
                if !tool_blocks.is_empty() {
                    match Message::builder()
                        .role(ConversationRole::User)
                        .set_content(Some(tool_blocks))
                        .build()
                    {
                        Ok(m) => bedrock_messages.push(m),
                        Err(e) => warn!("Failed to build tool result message: {e}"),
                    }
                }
            }
            LlmMessageRole::User => {
                let blocks = build_user_content(msg)?;
                if !blocks.is_empty() {
                    match Message::builder()
                        .role(ConversationRole::User)
                        .set_content(Some(blocks))
                        .build()
                    {
                        Ok(m) => bedrock_messages.push(m),
                        Err(e) => warn!("Failed to build user message: {e}"),
                    }
                }
                i += 1;
            }
            LlmMessageRole::Assistant => {
                let blocks = build_assistant_content(msg);
                if !blocks.is_empty() {
                    match Message::builder()
                        .role(ConversationRole::Assistant)
                        .set_content(Some(blocks))
                        .build()
                    {
                        Ok(m) => bedrock_messages.push(m),
                        Err(e) => warn!("Failed to build assistant message: {e}"),
                    }
                }
                i += 1;
            }
        }
    }

    let bedrock_messages = merge_consecutive_same_role(bedrock_messages);
    Ok((system_blocks, bedrock_messages))
}

fn build_user_content(msg: &LlmMessage) -> Result<Vec<ContentBlock>> {
    let mut blocks = Vec::new();
    match &msg.content {
        LlmMessageContent::Text(text) => {
            if !text.is_empty() {
                blocks.push(ContentBlock::Text(text.clone()));
            }
        }
        LlmMessageContent::Parts(parts) => {
            for part in parts {
                match part {
                    LlmContentPart::Text { text } => {
                        if !text.is_empty() {
                            blocks.push(ContentBlock::Text(text.clone()));
                        }
                    }
                    LlmContentPart::Image { url } => {
                        if let Some(block) = parse_image_url(url) {
                            blocks.push(block);
                        }
                    }
                    LlmContentPart::Audio { .. } => {
                        warn!("Audio content is not supported by Bedrock ConverseStream; skipping");
                    }
                }
            }
        }
    }
    Ok(blocks)
}

fn build_assistant_content(msg: &LlmMessage) -> Vec<ContentBlock> {
    let mut blocks = Vec::new();

    match &msg.content {
        LlmMessageContent::Text(text) if !text.is_empty() => {
            blocks.push(ContentBlock::Text(text.clone()));
        }
        LlmMessageContent::Parts(parts) => {
            for part in parts {
                if let LlmContentPart::Text { text } = part {
                    if !text.is_empty() {
                        blocks.push(ContentBlock::Text(text.clone()));
                    }
                }
            }
        }
        _ => {}
    }

    if let Some(calls) = &msg.tool_calls {
        for call in calls {
            let input_doc = json_to_document(call.arguments.clone());
            match ToolUseBlock::builder()
                .tool_use_id(&call.id)
                .name(&call.name)
                .input(input_doc)
                .build()
            {
                Ok(tu) => blocks.push(ContentBlock::ToolUse(tu)),
                Err(e) => warn!("Failed to build tool use block: {e}"),
            }
        }
    }

    blocks
}

fn build_tool_result_block(msg: &LlmMessage) -> Option<ContentBlock> {
    let tool_call_id = msg.tool_call_id.as_deref().unwrap_or("");
    let text = msg.content.to_text();

    let result_content = ToolResultContentBlock::Text(text);
    match ToolResultBlock::builder()
        .tool_use_id(tool_call_id)
        .content(result_content)
        .build()
    {
        Ok(tr) => Some(ContentBlock::ToolResult(tr)),
        Err(e) => {
            warn!("Failed to build tool result block: {e}");
            None
        }
    }
}

/// Merge consecutive messages with the same role.
/// Bedrock requires strictly alternating User/Assistant messages.
fn merge_consecutive_same_role(messages: Vec<Message>) -> Vec<Message> {
    let mut result: Vec<Message> = Vec::new();
    for msg in messages {
        if let Some(last) = result.last() {
            if last.role == msg.role {
                let last_idx = result.len() - 1;
                let prev = result.swap_remove(last_idx);
                let mut combined_content = prev.content.clone();
                combined_content.extend(msg.content.clone());
                match Message::builder()
                    .role(prev.role.clone())
                    .set_content(Some(combined_content))
                    .build()
                {
                    Ok(merged) => {
                        result.push(merged);
                        continue;
                    }
                    Err(_) => {
                        // Fall through: re-insert both
                        result.push(prev);
                    }
                }
            }
        }
        result.push(msg);
    }
    result
}

// ============================================================================
// Tool configuration
// ============================================================================

fn build_tool_config(tools: &[ToolDefinition]) -> Result<ToolConfiguration> {
    let mut tool_list = Vec::new();
    for tool in tools {
        let schema_doc = json_to_document(tool.parameters().clone());
        let spec = ToolSpecification::builder()
            .name(tool.name())
            .description(tool.description())
            .input_schema(ToolInputSchema::Json(schema_doc))
            .build()
            .map_err(|e| AgentLoopError::llm(format!("Failed to build tool spec: {e}")))?;
        tool_list.push(Tool::ToolSpec(spec));
    }
    ToolConfiguration::builder()
        .set_tools(Some(tool_list))
        .build()
        .map_err(|e| AgentLoopError::llm(format!("Failed to build tool config: {e}")))
}

// ============================================================================
// Image parsing
// ============================================================================

fn parse_image_url(url: &str) -> Option<ContentBlock> {
    if !url.starts_with("data:") {
        warn!("HTTP image URLs are not supported by Bedrock ConverseStream (use base64 data URLs)");
        return None;
    }

    let rest = url.strip_prefix("data:")?;
    let (mime_b64, data) = rest.split_once(',')?;
    let mime = mime_b64.split(';').next()?;

    let bytes = base64_decode(data)?;
    let format = match mime {
        "image/jpeg" | "image/jpg" => ImageFormat::Jpeg,
        "image/png" => ImageFormat::Png,
        "image/gif" => ImageFormat::Gif,
        "image/webp" => ImageFormat::Webp,
        other => {
            warn!("Unsupported image MIME type for Bedrock: {other}; skipping");
            return None;
        }
    };

    let source = ImageSource::Bytes(aws_sdk_bedrockruntime::primitives::Blob::new(bytes));
    let image_block = ImageBlock::builder()
        .format(format)
        .source(source)
        .build()
        .ok()?;

    Some(ContentBlock::Image(image_block))
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut table = [255u8; 256];
    for (i, &c) in CHARS.iter().enumerate() {
        table[c as usize] = i as u8;
    }

    let input: Vec<u8> = s
        .bytes()
        .filter(|&b| b != b'\n' && b != b'\r' && b != b' ' && b != b'=')
        .collect();

    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u8;

    for &c in &input {
        let val = table[c as usize];
        if val == 255 {
            return None; // Invalid character
        }
        buf = (buf << 6) | val as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

// ============================================================================
// JSON → aws_smithy_types::Document conversion
// ============================================================================

fn json_to_document(value: Value) -> Document {
    match value {
        Value::Null => Document::Null,
        Value::Bool(b) => Document::Bool(b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= 0 {
                    Document::Number(aws_smithy_types::Number::PosInt(i as u64))
                } else {
                    Document::Number(aws_smithy_types::Number::NegInt(i))
                }
            } else if let Some(f) = n.as_f64() {
                Document::Number(aws_smithy_types::Number::Float(f))
            } else {
                Document::Null
            }
        }
        Value::String(s) => Document::String(s),
        Value::Array(arr) => Document::Array(arr.into_iter().map(json_to_document).collect()),
        Value::Object(obj) => Document::Object(
            obj.into_iter()
                .map(|(k, v)| (k, json_to_document(v)))
                .collect(),
        ),
    }
}

// ============================================================================
// Error classification
// ============================================================================

fn is_too_large(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("too long")
        || lower.contains("too large")
        || lower.contains("context length")
        || lower.contains("maximum tokens")
        || lower.contains("input is too long")
        || lower.contains("prompt is too long")
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_too_large_detects_bedrock_messages() {
        assert!(is_too_large("ValidationException: Input is too long"));
        assert!(is_too_large("maximum tokens exceeded"));
        assert!(is_too_large("prompt is too long for this model"));
        assert!(!is_too_large("authentication failed"));
        assert!(!is_too_large("model not found"));
    }

    #[test]
    fn test_json_to_document_types() {
        assert!(matches!(json_to_document(Value::Null), Document::Null));
        assert!(matches!(
            json_to_document(Value::Bool(true)),
            Document::Bool(true)
        ));
        assert!(matches!(
            json_to_document(serde_json::json!(42)),
            Document::Number(aws_smithy_types::Number::PosInt(42))
        ));
        assert!(matches!(
            json_to_document(serde_json::json!(-1)),
            Document::Number(aws_smithy_types::Number::NegInt(-1))
        ));
        assert!(matches!(
            json_to_document(Value::String("hello".to_string())),
            Document::String(s) if s == "hello"
        ));
    }

    #[test]
    fn test_base64_decode_valid() {
        let encoded = "SGVsbG8gV29ybGQ=";
        let decoded = base64_decode(encoded).unwrap();
        assert_eq!(decoded, b"Hello World");
    }

    #[test]
    fn test_base64_decode_invalid_returns_none() {
        assert!(base64_decode("!@#$").is_none());
    }
}
