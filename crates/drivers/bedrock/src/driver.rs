// AWS Bedrock Runtime Chat Driver
//
// Implements ChatDriver using the AWS Bedrock Runtime ConverseStream API.
// Uses the aws-sdk-bedrockruntime crate for typed event stream handling,
// which handles SigV4 signing and binary event stream framing internally.
//
// Credential shape: typed credential fields (see credential.rs).
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
use base64::prelude::*;
use everruns_provider::credential_schema::{CredentialFormSchema, FormField};
use everruns_provider::driver_registry::{
    BoxedChatDriver, ChatDriver, DiscoveredModel, DriverConfig, DriverDescriptor, DriverId,
    DriverRegistry, LlmCallConfig, LlmCompletionMetadata, LlmContentPart, LlmMessage,
    LlmMessageContent, LlmMessageRole, LlmResponseStream, LlmStreamEvent,
};
use everruns_provider::error::{AgentLoopError, LlmErrorKind, Result};
use everruns_provider::tool_types::{ToolCall, ToolDefinition};
use serde_json::Value;
use std::collections::HashMap;
use tokio_stream::wrappers::ReceiverStream;
use tracing::warn;

const BEDROCK_STREAM_BUFFER_SIZE: usize = 64;

use crate::credential::BedrockCredential;

/// Provider-owned Bedrock authentication and signing stack.
///
/// The AWS SDK resolves credentials and signs every ConverseStream request;
/// keeping its client here preserves SigV4/event-stream behavior while the
/// protocol driver itself remains credential-free.
#[derive(Clone)]
pub struct BedrockAuth {
    client: Client,
}

impl BedrockAuth {
    pub fn new(credential: BedrockCredential) -> Self {
        Self {
            client: build_client(&credential),
        }
    }

    pub fn from_config(config: &DriverConfig) -> Result<Self> {
        let credential = BedrockCredential::from_driver_config(config)?;
        Ok(Self::new(credential))
    }
}

impl std::fmt::Debug for BedrockAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BedrockAuth")
            .field("configured", &true)
            .finish()
    }
}

#[async_trait]
impl everruns_provider::ProviderAuth for BedrockAuth {
    async fn headers(
        &self,
        _request: everruns_provider::ProviderAuthRequest<'_>,
    ) -> Result<Vec<(String, String)>> {
        Ok(Vec::new())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Credential-free Bedrock ConverseStream wire-protocol driver.
#[derive(Clone, Debug, Default)]
pub struct BedrockChatDriver;

impl BedrockChatDriver {
    pub fn new() -> Self {
        Self
    }
}

/// Ready-to-use AWS Bedrock provider assembly.
pub fn provider(
    id: impl Into<everruns_provider::ProviderKey>,
    credential: BedrockCredential,
) -> everruns_provider::Provider {
    everruns_provider::Provider::new(id, BedrockChatDriver::new())
        .auth(BedrockAuth::new(credential))
}

/// Build the AWS Bedrock runtime client from typed credentials. Called once per
/// driver construction, not per request.
fn build_client(credential: &BedrockCredential) -> Client {
    let creds = Credentials::new(
        credential.access_key_id.clone(),
        credential.secret_access_key.clone(),
        credential.session_token.clone(),
        None,
        "everruns-bedrock",
    );
    let config = BedrockConfigBuilder::new()
        .behavior_version(BehaviorVersion::latest())
        .credentials_provider(creds)
        .region(Region::new(credential.region.clone()))
        .build();
    Client::from_conf(config)
}

/// Register the Bedrock driver with the given registry.
pub fn register_driver(registry: &mut DriverRegistry) {
    // Bedrock's multi-field credential is a declared schema of discrete typed
    // fields (knowledge/foundations/providers.md), not a JSON document smuggled through
    // `api_key`. The fields are assembled into the stored credential document
    // and parsed back into the typed `DriverConfig` credential map.
    registry.register_descriptor(DriverDescriptor {
        display_name: "AWS Bedrock".into(),
        credential_schema: CredentialFormSchema {
            fields: vec![
                FormField::password("access_key_id", "Access Key ID").required(),
                FormField::password("secret_access_key", "Secret Access Key").required(),
                // Optional to match BedrockCredential, which defaults the
                // region to us-east-1 when omitted.
                FormField::text("region", "Region")
                    .with_placeholder("us-east-1")
                    .with_default("us-east-1")
                    .with_help("Defaults to us-east-1."),
                FormField::password("session_token", "Session Token")
                    .with_help("Only for temporary credentials."),
            ],
            instructions_markdown:
                "Create an IAM user or role with Bedrock invoke permissions and use its access keys."
                    .to_string(),
        },
        ..DriverDescriptor::chat_only(DriverId::Bedrock, |config| {
            match BedrockAuth::from_config(config) {
                Ok(auth) => everruns_provider::Provider::new(
                    config.provider.clone(),
                    BedrockChatDriver::new(),
                )
                .auth(auth)
                .into_boxed_driver(),
                Err(e) => Box::new(FailDriver(e.to_string())) as BoxedChatDriver,
            }
        })
    });
}

/// Driver that immediately fails with a credential error.
struct FailDriver(String);

#[async_trait]
impl ChatDriver for FailDriver {
    async fn chat_completion_stream(
        &self,
        _endpoint: &everruns_provider::ProviderEndpoint,
        _messages: Vec<LlmMessage>,
        _config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        Err(AgentLoopError::llm(self.0.clone()))
    }
}

#[async_trait]
impl ChatDriver for BedrockChatDriver {
    async fn chat_completion_stream(
        &self,
        endpoint: &everruns_provider::ProviderEndpoint,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        let client = endpoint
            .auth::<BedrockAuth>()
            .ok_or_else(|| AgentLoopError::config("Bedrock provider requires BedrockAuth"))?
            .client
            .clone();
        let model_id = config.model.clone();

        let (system_blocks, bedrock_messages) = build_messages(&messages)?;

        let tool_cfg = if !config.tools.is_empty() {
            Some(build_tool_config(&config.tools)?)
        } else {
            None
        };

        let inference_cfg = InferenceConfiguration::builder()
            .set_temperature(config.temperature)
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
                // No HTTP status at this boundary; classify the semantic
                // kind from AWS SDK exception names in the message text.
                AgentLoopError::llm_kind(
                    LlmErrorKind::from_error_text(&msg),
                    format!("Bedrock ConverseStream failed: {e}"),
                )
            }
        })?;

        let mut event_stream = response.stream;
        let (tx, rx) =
            tokio::sync::mpsc::channel::<Result<LlmStreamEvent>>(BEDROCK_STREAM_BUFFER_SIZE);

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
                                    if tx
                                        .send(Ok(LlmStreamEvent::TextDelta(t.clone())))
                                        .await
                                        .is_err()
                                    {
                                        return;
                                    }
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
                                let result: Result<Vec<ToolCall>> = ordered
                                    .into_iter()
                                    .map(|(_, ptc)| {
                                        let arguments = serde_json::from_str(&ptc.input_json)
                                            .map_err(|e| {
                                                AgentLoopError::llm(format!(
                                                    "invalid Bedrock tool arguments JSON: {e}"
                                                ))
                                            })?;
                                        Ok(ToolCall {
                                            id: ptc.id,
                                            name: ptc.name,
                                            arguments,
                                        })
                                    })
                                    .collect();
                                match result {
                                    Ok(calls) => {
                                        if tx
                                            .send(Ok(LlmStreamEvent::ToolCalls(calls)))
                                            .await
                                            .is_err()
                                        {
                                            return;
                                        }
                                    }
                                    Err(e) => {
                                        let _ = tx.send(Err(e)).await;
                                        return;
                                    }
                                }
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
                        let _ = tx.send(Ok(LlmStreamEvent::Done(Box::new(meta)))).await;
                        return;
                    }
                    Err(e) => {
                        let msg = format!("{e}");
                        let err = if is_too_large(&msg) {
                            AgentLoopError::request_too_large(msg)
                        } else {
                            AgentLoopError::llm_kind(
                                LlmErrorKind::from_error_text(&msg),
                                format!("Bedrock stream error: {e}"),
                            )
                        };
                        let _ = tx.send(Err(err)).await;
                        return;
                    }
                }
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn list_models(
        &self,
        _endpoint: &everruns_provider::ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>> {
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
                            if let LlmContentPart::Text { text } = part
                                && !text.is_empty()
                            {
                                system_blocks.push(SystemContentBlock::Text(text.clone()));
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
                    let m = Message::builder()
                        .role(ConversationRole::User)
                        .set_content(Some(tool_blocks))
                        .build()
                        .map_err(|e| {
                            AgentLoopError::llm(format!("Failed to build Bedrock message: {e}"))
                        })?;
                    bedrock_messages.push(m);
                }
            }
            LlmMessageRole::User => {
                let blocks = build_user_content(msg)?;
                if !blocks.is_empty() {
                    let m = Message::builder()
                        .role(ConversationRole::User)
                        .set_content(Some(blocks))
                        .build()
                        .map_err(|e| {
                            AgentLoopError::llm(format!("Failed to build Bedrock message: {e}"))
                        })?;
                    bedrock_messages.push(m);
                }
                i += 1;
            }
            LlmMessageRole::Assistant => {
                let blocks = build_assistant_content(msg);
                if !blocks.is_empty() {
                    let m = Message::builder()
                        .role(ConversationRole::Assistant)
                        .set_content(Some(blocks))
                        .build()
                        .map_err(|e| {
                            AgentLoopError::llm(format!("Failed to build Bedrock message: {e}"))
                        })?;
                    bedrock_messages.push(m);
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
                if let LlmContentPart::Text { text } = part
                    && !text.is_empty()
                {
                    blocks.push(ContentBlock::Text(text.clone()));
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
    if tool_call_id.is_empty() {
        warn!("Tool message is missing tool_call_id; skipping tool result block");
        return None;
    }
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
        if let Some(last) = result.last()
            && last.role == msg.role
        {
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

    let bytes = BASE64_STANDARD.decode(data).ok()?;
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
            } else if let Some(u) = n.as_u64() {
                // THREAT[TM-TOOL-039]: identifiers above i64::MAX must not round through f64.
                Document::Number(aws_smithy_types::Number::PosInt(u))
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
    #[test]
    fn registered_descriptor_declares_aws_credential_fields() {
        let mut registry = DriverRegistry::new();
        super::register_driver(&mut registry);

        let descriptor = registry.descriptor(&DriverId::Bedrock).unwrap();
        assert_eq!(descriptor.display_name, "AWS Bedrock");
        let names: Vec<&str> = descriptor
            .credential_schema
            .fields
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "access_key_id",
                "secret_access_key",
                "region",
                "session_token"
            ]
        );
        // Region (defaulted to us-east-1 at parse time) and session token are
        // optional; the key pair is required.
        let required: Vec<bool> = descriptor
            .credential_schema
            .fields
            .iter()
            .map(|f| f.required)
            .collect();
        assert_eq!(required, [true, true, false, false]);
    }

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
    fn messages_preserve_full_system_conversation_and_tool_result_order() {
        let mut multipart = LlmMessage::text(LlmMessageRole::System, "");
        multipart.content = LlmMessageContent::Parts(vec![
            LlmContentPart::Text { text: "B".into() },
            LlmContentPart::Text { text: "".into() },
            LlmContentPart::Text { text: "C".into() },
        ]);
        let mut call = LlmMessage::text(LlmMessageRole::Assistant, "calling");
        call.tool_calls = Some(vec![ToolCall {
            id: "call-one".into(),
            name: "inspect".into(),
            arguments: serde_json::json!({"path":"a"}),
        }]);
        let mut result = LlmMessage::text(LlmMessageRole::Tool, "result-one");
        result.tool_call_id = Some("call-one".into());
        let mut next_result = LlmMessage::text(LlmMessageRole::Tool, "result-two");
        next_result.tool_call_id = Some("call-two".into());
        let mut blank_id = LlmMessage::text(LlmMessageRole::Tool, "discard-empty-id");
        blank_id.tool_call_id = Some(String::new());
        let input = vec![
            LlmMessage::text(LlmMessageRole::System, "A"),
            LlmMessage::text(LlmMessageRole::User, "hello"),
            multipart,
            LlmMessage::text(LlmMessageRole::User, "world"),
            call,
            result,
            LlmMessage::text(LlmMessageRole::Tool, "discard-no-id"),
            blank_id,
            next_result,
            LlmMessage::text(LlmMessageRole::User, "follow-up"),
            LlmMessage::text(LlmMessageRole::Assistant, "done"),
            LlmMessage::text(LlmMessageRole::User, "last"),
        ];
        let (system, messages) = build_messages(&input).unwrap();
        assert_eq!(
            system,
            vec![
                SystemContentBlock::Text("A".into()),
                SystemContentBlock::Text("B".into()),
                SystemContentBlock::Text("C".into())
            ]
        );
        let message = |role, content| {
            Message::builder()
                .role(role)
                .set_content(Some(content))
                .build()
                .unwrap()
        };
        assert_eq!(
            messages,
            vec![
                message(
                    ConversationRole::User,
                    vec![
                        ContentBlock::Text("hello".into()),
                        ContentBlock::Text("world".into())
                    ]
                ),
                message(
                    ConversationRole::Assistant,
                    vec![
                        ContentBlock::Text("calling".into()),
                        ContentBlock::ToolUse(
                            ToolUseBlock::builder()
                                .tool_use_id("call-one")
                                .name("inspect")
                                .input(Document::Object(HashMap::from([(
                                    "path".into(),
                                    Document::String("a".into())
                                )])))
                                .build()
                                .unwrap()
                        )
                    ]
                ),
                message(
                    ConversationRole::User,
                    vec![
                        ContentBlock::ToolResult(
                            ToolResultBlock::builder()
                                .tool_use_id("call-one")
                                .content(ToolResultContentBlock::Text("result-one".into()))
                                .build()
                                .unwrap()
                        ),
                        ContentBlock::ToolResult(
                            ToolResultBlock::builder()
                                .tool_use_id("call-two")
                                .content(ToolResultContentBlock::Text("result-two".into()))
                                .build()
                                .unwrap()
                        ),
                        ContentBlock::Text("follow-up".into())
                    ]
                ),
                message(
                    ConversationRole::Assistant,
                    vec![ContentBlock::Text("done".into())]
                ),
                message(
                    ConversationRole::User,
                    vec![ContentBlock::Text("last".into())]
                ),
            ]
        );
        assert_eq!(build_messages(&[]).unwrap(), (vec![], vec![]));
    }
    #[tokio::test]
    async fn tool_arguments_preserve_nested_documents_and_unsigned_integer_precision() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::builder().start().await;
        Mock::given(method("POST"))
            .and(path("/model/model/converse-stream"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/vnd.amazon.eventstream")
                    .set_body_bytes(vec![]),
            )
            .expect(1)
            .mount(&server)
            .await;
        let client = Client::from_conf(
            BedrockConfigBuilder::new()
                .behavior_version(BehaviorVersion::latest())
                .region(Region::new("us-east-1"))
                .credentials_provider(Credentials::new(
                    "synthetic-access",
                    "synthetic-secret",
                    Some("synthetic-session".into()),
                    None,
                    "test",
                ))
                .endpoint_url(server.uri())
                .build(),
        );
        let service = everruns_provider::Provider::new("bedrock", BedrockChatDriver::new())
            .auth(BedrockAuth { client });
        let arguments = serde_json::json!({"values":[null,true,"hé🙂",42,-1,1.5,18446744073709551615_u64],"nested":{"id":9223372036854775809_u64}});
        let mut message = LlmMessage::text(LlmMessageRole::Assistant, "");
        message.tool_calls = Some(vec![ToolCall {
            id: "exact-id".into(),
            name: "inspect".into(),
            arguments: arguments.clone(),
        }]);
        let config = LlmCallConfig {
            model: "model".into(),
            temperature: Some(0.25),
            max_tokens: Some(32),
            tools: vec![],
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            metadata: Default::default(),
            previous_response_id: None,
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            openrouter_routing: None,
            parallel_tool_calls: None,
            volatile_suffix_len: 0,
            extra_headers: vec![],
            cache_diagnostics: None,
        };
        let response = service
            .chat_completion(vec![message], &config)
            .await
            .unwrap();
        assert!(response.text.is_empty());
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0]
                .headers
                .get("x-amz-security-token")
                .unwrap()
                .to_str()
                .unwrap(),
            "synthetic-session"
        );
        assert!(
            requests[0]
                .headers
                .get("authorization")
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("AWS4-HMAC-SHA256 Credential=synthetic-access/")
        );
        assert_eq!(
            requests[0].body_json::<Value>().unwrap(),
            serde_json::json!({"messages":[{"role":"assistant","content":[{"toolUse":{"toolUseId":"exact-id","name":"inspect","input":arguments}}]}],"inferenceConfig":{"temperature":0.25,"maxTokens":32}})
        );
    }
}
