//! Native Converse completion, sharing the streamed tool-settlement rules.
use super::*;

/// Fold a whole Converse response with the same tool-settlement and accounting
/// rules as ConverseStream. Authentication and retries stay in the AWS SDK.
pub(super) fn native_converse_events(
    response: aws_sdk_bedrockruntime::operation::converse::ConverseOutput,
    model: &str,
) -> Result<LlmResponseStream> {
    let message = match response.output() {
        Some(aws_sdk_bedrockruntime::types::ConverseOutput::Message(message)) => message,
        _ => return Err(AgentLoopError::llm("Bedrock Converse returned no message")),
    };
    let mut events = Vec::new();
    let mut pending = HashMap::new();
    let mut stopped = HashSet::new();
    for (index, block) in message.content().iter().enumerate() {
        match block {
            ContentBlock::Text(text) => events.push(Ok(LlmStreamEvent::TextDelta(text.clone()))),
            ContentBlock::ToolUse(call) => {
                pending.insert(
                    index,
                    PartialToolCall {
                        id: call.tool_use_id().to_string(),
                        name: call.name().to_string(),
                        input_json: document_to_json(call.input()).to_string(),
                    },
                );
                stopped.insert(index);
            }
            _ => {}
        }
    }
    let raw = response.stop_reason().as_str();
    let settled = settle_tool_calls(pending, &stopped, raw);
    llm_telemetry::warn_tool_calls_dropped("bedrock", model, settled.dropped, raw);
    llm_telemetry::warn_tool_calls_truncated_executed(
        "bedrock",
        model,
        settled.truncated_executed,
        raw,
    );
    if !settled.calls.is_empty() {
        events.push(Ok(LlmStreamEvent::ToolCalls(settled.calls)));
    }
    let mut metadata = LlmCompletionMetadata::default();
    metadata.model = Some(model.to_string());
    metadata.finish_reason = Some(normalize_stop_reason(raw));
    metadata.provider_finish_reason = Some(raw.to_string());
    metadata.tool_calls_dropped = settled.dropped;
    metadata.tool_calls_truncated_executed = settled.truncated_executed;
    if let Some(usage) = response.usage() {
        let input = usage.input_tokens() as u32;
        let output = usage.output_tokens() as u32;
        let read = usage.cache_read_input_tokens().map(|tokens| tokens as u32);
        let write = usage.cache_write_input_tokens().map(|tokens| tokens as u32);
        metadata.prompt_tokens = Some(input);
        metadata.completion_tokens = Some(output);
        metadata.cache_read_tokens = read;
        metadata.cache_creation_tokens = write;
        metadata.cache_creation_1h_tokens = usage
            .cache_details()
            .iter()
            .find(|detail| detail.ttl() == &CacheTtl::OneHour)
            .map(|detail| detail.input_tokens() as u32);
        metadata.total_tokens = Some(input + output + read.unwrap_or(0) + write.unwrap_or(0));
    }
    events.push(Ok(LlmStreamEvent::Done(Box::new(metadata))));
    Ok(Box::pin(futures::stream::iter(events)))
}

fn document_to_json(document: &Document) -> Value {
    match document {
        Document::Null => Value::Null,
        Document::Bool(value) => Value::Bool(*value),
        Document::String(value) => Value::String(value.clone()),
        Document::Number(aws_smithy_types::Number::PosInt(value)) => (*value).into(),
        Document::Number(aws_smithy_types::Number::NegInt(value)) => (*value).into(),
        Document::Number(aws_smithy_types::Number::Float(value)) => {
            serde_json::Number::from_f64(*value)
                .map(Value::Number)
                .unwrap_or(Value::Null)
        }
        Document::Array(values) => Value::Array(values.iter().map(document_to_json).collect()),
        Document::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), document_to_json(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn native_converse_preserves_tool_arguments_and_cache_accounting() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/model/model/converse"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "output":{"message":{"role":"assistant","content":[{"text":"answer"},{"toolUse":{"toolUseId":"call-native","name":"lookup","input":{"id":18446744073709551615_u64}}}]}},
                "stopReason":"tool_use", "usage":{"inputTokens":70,"outputTokens":20,"totalTokens":130,"cacheReadInputTokens":30,"cacheWriteInputTokens":10},"metrics":{"latencyMs":1}
            }))).expect(1).mount(&server).await;
        let client = Client::from_conf(
            BedrockConfigBuilder::new()
                .behavior_version(BehaviorVersion::latest())
                .region(Region::new("us-east-1"))
                .credentials_provider(Credentials::new(
                    "synthetic-access",
                    "synthetic-secret",
                    None,
                    None,
                    "test",
                ))
                .endpoint_url(server.uri())
                .build(),
        );
        let provider = everruns_contracts::Provider::new("test", BedrockChatDriver::new())
            .auth(BedrockAuth { client });
        let result = provider
            .chat_completion_non_streaming(
                vec![Message::text(MessageRole::User, "work")],
                &LlmCallConfig::new("model"),
            )
            .await
            .unwrap();
        assert_eq!(result.text, "answer");
        assert_eq!(
            result.tool_calls.unwrap()[0].arguments,
            serde_json::json!({"id":18446744073709551615_u64})
        );
        assert_eq!(result.metadata.prompt_tokens, Some(70));
        assert_eq!(result.metadata.completion_tokens, Some(20));
        assert_eq!(result.metadata.cache_read_tokens, Some(30));
        assert_eq!(result.metadata.cache_creation_tokens, Some(10));
        assert_eq!(result.metadata.total_tokens, Some(130));
        assert_eq!(result.metadata.finish_reason.as_deref(), Some("tool_calls"));
    }
}
