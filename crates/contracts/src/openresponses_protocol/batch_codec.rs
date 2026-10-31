//! Shared request assembly and offline response decoding.
use super::*;
use crate::{
    ChatDriver, LlmResponse, LlmStreamEvent,
    turn_collector::{TurnLimits, collect_turn},
};

impl OpenResponsesProtocolChatDriver {
    pub(super) fn completion_request(
        &self,
        config: &LlmCallConfig,
        input_items: Vec<ResponsesInputItem>,
        instructions: Option<String>,
        previous_response_id: Option<String>,
        tools: Option<Vec<ResponsesTool>>,
        has_update_state: bool,
    ) -> ResponsesRequest {
        // Build reasoning config if specified.
        // Skip when effort is "none" — sending reasoning params to models that
        // don't support them (or with effort=none) causes OpenAI API errors.
        let reasoning = config
            .reasoning_effort
            .filter(crate::model::ReasoningEffort::requests_reasoning)
            .map(|effort| ResponsesReasoning {
                effort: effort.as_str().to_string(),
                summary: "detailed".to_string(),
            });

        // Reasoning items are only replayable when the provider hands back
        // their encrypted payload, and it only does so on request. Stateful
        // continuations already retain that state server-side; Meta rejects
        // this include when paired with `previous_response_id`.
        let should_include_reasoning = reasoning.is_some() || has_update_state;
        let include = previous_response_id
            .is_none()
            .then_some(should_include_reasoning)
            .filter(|include| *include)
            .map(|_| vec!["reasoning.encrypted_content".to_string()]);

        // Build metadata for request tracking
        let metadata = if config.metadata.is_empty() {
            None
        } else {
            Some(config.metadata.clone())
        };
        let prompt_cache_key =
            Self::build_prompt_cache_key(config, &input_items, &instructions, &tools);
        ResponsesRequest {
            model: config.model.clone(),
            input: input_items,
            instructions,
            previous_response_id,
            temperature: config.temperature,
            max_output_tokens: config.max_tokens,
            stream: true,
            tools,
            reasoning,
            metadata,
            prompt_cache_key,
            parallel_tool_calls: config
                .resolved_parallel_tool_calls(self.supports_parallel_tool_calls(&config.model)),
            service_tier: config.speed.clone(),
            text: ResponsesText {
                verbosity: config.verbosity.clone(),
                format: config
                    .response_format
                    .as_ref()
                    .map(|f| f.responses_text_format()),
            }
            .into_option(),
            include,
        }
    }
    /// Encode a stateless non-streaming Responses request for a native batch.
    pub fn encode_batch_request(
        &self,
        messages: &[Message],
        config: &LlmCallConfig,
    ) -> Result<Value> {
        crate::openai_compat::validate_config(config)?;
        let (instructions, input) =
            Self::build_input_with_changes(messages, self.native_phases, None);
        // Tickets do not retain original schemas: preserve optional function
        // arguments so collection after restart needs no nullable restoration.
        let tools = (!config.tools.is_empty()).then(|| {
            config
                .tools
                .iter()
                .map(|tool| Self::function_tool_mode(tool, None, false))
                .collect()
        });
        let mut request = self.completion_request(config, input, instructions, None, tools, false);
        request.stream = false;
        serde_json::to_value(request)
            .map_err(|_| AgentLoopError::config("batch request encoding failed"))
    }
    /// Decode a complete Responses resource without network or tool execution.
    pub async fn decode_batch_response(&self, value: Value) -> Result<LlmResponse> {
        if !value["output"].is_array() {
            return Err(AgentLoopError::llm("missing batch output"));
        }
        let response: types::ResponseResource = serde_json::from_value(value)
            .map_err(|_| AgentLoopError::llm("invalid batch response"))?;
        if !matches!(
            response.status,
            types::ResponseStatus::Completed | types::ResponseStatus::Incomplete
        ) {
            return Err(AgentLoopError::llm("nonterminal batch response"));
        }
        let input = Mutex::new(0);
        let output = Mutex::new(0);
        let cache = Mutex::new(None);
        let calls = Mutex::new(ToolCallStream::default());
        let finish = Mutex::new(None);
        let deferred = Mutex::new(Vec::new());
        let mut events = Vec::new();
        for item in &response.output {
            match item {
                types::OutputItem::Message { content, .. } => {
                    for part in content {
                        match part {
                            types::ContentPart::OutputText { text, .. }
                            | types::ContentPart::Text { text } => {
                                events.push(LlmStreamEvent::TextDelta(text.clone()))
                            }
                            types::ContentPart::Refusal { .. } => {
                                return Err(AgentLoopError::llm("batch response refused"));
                            }
                            _ => {}
                        }
                    }
                }
                types::OutputItem::Reasoning { .. } => events.push(handle_streaming_event(
                    types::StreamingEvent::OutputItemDone {
                        sequence_number: 0,
                        output_index: 0,
                        item: Some(item.clone()),
                    },
                    &input,
                    &output,
                    &cache,
                    &calls,
                    &finish,
                    &deferred,
                    response.model.clone(),
                    None,
                )),
                _ => {}
            }
        }
        let model = response.model.clone();
        let event = if response.status == types::ResponseStatus::Incomplete {
            types::StreamingEvent::ResponseIncomplete {
                sequence_number: 0,
                response,
            }
        } else {
            types::StreamingEvent::ResponseCompleted {
                sequence_number: 0,
                response,
            }
        };
        let done = handle_streaming_event(
            event, &input, &output, &cache, &calls, &finish, &deferred, model, None,
        );
        events.extend(
            deferred
                .into_inner()
                .map_err(|_| AgentLoopError::llm("batch decoder state failed"))?,
        );
        events.push(done);
        collect_turn(
            Box::pin(futures::stream::iter(events.into_iter().map(Ok))),
            &TurnLimits::default(),
            |_| {},
        )
        .await
        .map(|turn| turn.into_response())
    }
}
