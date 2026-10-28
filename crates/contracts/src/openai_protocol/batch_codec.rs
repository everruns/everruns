//! Offline wire codec shares request conversion and response decoding with chat.
use super::*;
use serde_json::Value;

impl OpenAIProtocolChatDriver {
    pub(super) fn completion_request(
        &self,
        api_url: &str,
        messages: &[Message],
        config: &LlmCallConfig,
        strict: bool,
    ) -> Result<OpenAiRequest> {
        let openai_messages: Vec<OpenAiMessage> = convert_messages(messages);
        let tools = if config.tools.is_empty() {
            None
        } else {
            Some(Self::convert_tools_mode(&config.tools, strict))
        };
        let metadata = if config.metadata.is_empty() {
            None
        } else {
            Some(config.metadata.clone())
        };
        let (max_tokens, max_completion_tokens) = max_output_fields(api_url, config.max_tokens);
        Ok(OpenAiRequest {
            model: config.model.clone(),
            messages: openai_messages,
            temperature: config.temperature,
            max_tokens,
            max_completion_tokens,
            stream: false,
            stream_options: None,
            tools,
            parallel_tool_calls: config
                .resolved_parallel_tool_calls(self.supports_parallel_tool_calls(&config.model)),
            reasoning_effort: OpenAiRequest::reasoning_effort_for(config),
            service_tier: config.speed.clone(),
            verbosity: config.verbosity.clone(),
            metadata,
            response_format: OpenAiRequest::response_format_for(config),
            prompt_cache_key: self.prompt_cache_key_for(config),
        })
    }
    pub(super) fn decode_completion(
        &self,
        body: OpenAiChatCompletionResponse,
        config: &LlmCallConfig,
    ) -> Result<LlmResponse> {
        if let Some(error) = body.error {
            return Err(error.into_stream_error().into_agent_error());
        }
        let response_model = body.model.clone();
        let (text, mut tool_calls, reasoning, finish_reason) = match body.choices.into_iter().next()
        {
            Some(mut choice) => {
                let (text, thinking) = choice.message.text_and_thinking();
                // Mirror the streaming path: tool calls whose arguments are not
                // complete JSON values cannot execute, so drop them.
                let tool_calls: Vec<ToolCall> = choice
                    .message
                    .tool_calls
                    .into_iter()
                    .filter_map(|tool_call| {
                        let arguments = serde_json::from_str(&tool_call.function.arguments).ok()?;
                        Some(ToolCall {
                            id: tool_call.id,
                            name: tool_call.function.name,
                            arguments,
                        })
                    })
                    .collect();
                let reasoning = thinking.map(|text| {
                    crate::reasoning::ReasoningContentPart::opaque("openai-protocol")
                        .with_text(crate::reasoning::ReasoningText::Plain { text })
                });
                (text, tool_calls, reasoning, choice.finish_reason)
            }
            None => (String::new(), Vec::new(), None, None),
        };
        crate::tool_schema_compat::restore_strict_openai_tool_calls(&config.tools, &mut tool_calls);
        let (prompt_tokens, completion_tokens, cached_tokens, reasoning_tokens, cost) = body
            .usage
            .map(|usage| {
                let cached = usage
                    .prompt_tokens_details
                    .as_ref()
                    .and_then(|details| details.cached_tokens);
                let reasoning = usage
                    .completion_tokens_details
                    .as_ref()
                    .and_then(|details| details.reasoning_tokens);
                let prompt = usage.prompt_tokens.unwrap_or(0);
                (
                    Some(disjoint_prompt_tokens(prompt, cached)),
                    usage.completion_tokens,
                    cached,
                    reasoning,
                    usage.cost,
                )
            })
            .unwrap_or((None, None, None, None, None));
        Ok(LlmResponse {
            text,
            reasoning: reasoning.into_iter().collect(),
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            metadata: LlmCompletionMetadata {
                total_tokens: prompt_tokens
                    .unwrap_or(0)
                    .checked_add(cached_tokens.unwrap_or(0))
                    .and_then(|prompt| prompt.checked_add(completion_tokens.unwrap_or(0))),
                prompt_tokens,
                completion_tokens,
                cache_read_tokens: cached_tokens,
                reasoning_tokens,
                provider_cost_usd: cost,
                model: Some(config.model.clone()),
                response_model,
                // Chat Completions reasons are already the normalized vocabulary.
                provider_finish_reason: finish_reason.clone(),
                finish_reason,
                response_id: body.id,
                ..Default::default()
            },
        })
    }
    /// Encode a stateless non-streaming Chat Completions request for a native batch.
    pub fn encode_batch_request(
        &self,
        messages: &[Message],
        config: &LlmCallConfig,
    ) -> Result<Value> {
        crate::openai_compat::validate_config(config)?;
        // Tickets retain IDs, not tool definitions. Preserve optional arguments
        // on the wire so collection after restart needs no schema restoration.
        serde_json::to_value(self.completion_request(
            "https://api.openai.com/v1/chat/completions",
            messages,
            config,
            false,
        )?)
        .map_err(|_| AgentLoopError::config("batch request encoding failed"))
    }
    /// Decode one native batch response without network requests or tool execution.
    pub fn decode_batch_response(&self, value: Value) -> Result<LlmResponse> {
        if value["choices"].as_array().is_none_or(|c| c.is_empty()) {
            return Err(AgentLoopError::llm("missing batch choices"));
        }
        let body: OpenAiChatCompletionResponse = serde_json::from_value(value)
            .map_err(|_| AgentLoopError::llm("invalid batch completion"))?;
        let config = LlmCallConfig::new(body.model.clone().unwrap_or_default());
        self.decode_completion(body, &config)
    }
}
