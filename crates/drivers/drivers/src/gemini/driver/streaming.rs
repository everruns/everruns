//! Streaming conversion and bounded long-decode continuation.

use super::*;

pub(super) struct ContinuationRequest {
    pub(super) driver: GeminiChatDriver,
    pub(super) endpoint: everruns_contracts::ProviderEndpoint,
    pub(super) request: GeminiRequest,
    pub(super) url: String,
    pub(super) model: String,
    pub(super) extra_headers: Vec<(String, String)>,
    pub(super) hops: u32,
}

impl ContinuationRequest {
    async fn next(&mut self, token: String) -> Result<(ByteStream, RetryMetadata)> {
        // THREAT[TM-DOS-044]: malformed provider progress must not cause an
        // unbounded request loop. 256 covers the 1M window's 5K checkpoints.
        if self.hops >= 256 || self.request.continuation_token.as_ref() == Some(&token) {
            return Err(AgentLoopError::llm("Gemini continuation made no progress"));
        }
        self.hops += 1;
        self.request.continuation_token = Some(token);
        connect_bytes_with_reconnect(&self.driver.retry_config, "GeminiDriver", |_attempt| {
            self.driver.send_generate_content_request(
                &self.endpoint,
                &self.request,
                &self.url,
                &self.model,
                &self.extra_headers,
            )
        })
        .await
    }
}

pub(super) fn convert_gemini_stream(
    byte_stream: ByteStream,
    state: GeminiStreamState,
    continuation: Option<ContinuationRequest>,
) -> LlmResponseStream {
    // THREAT[TM-TOOL-038]: decode UTF-8 and SSE framing before parsing provider JSON. Transport chunks
    // may split a character, line or event anywhere.
    let event_stream = byte_stream.eventsource();
    Box::pin(futures::stream::unfold(
        (event_stream, state, continuation),
        |(mut stream, mut state, mut continuation)| async move {
            loop {
                if let Some(event) = state.pending.pop_front() {
                    return Some((Ok(event), (stream, state, continuation)));
                }
                if state.done {
                    return None;
                }
                let error = match stream.next().await {
                    Some(Ok(event)) if event.data == "[DONE]" => None,
                    Some(Ok(event)) => {
                        match serde_json::from_str::<GeminiStreamResponse>(&event.data) {
                            Ok(response) => state.response(response),
                            Err(error) => {
                                tracing::debug!(%error, "GeminiDriver: failed to parse SSE event")
                            }
                        }
                        continue;
                    }
                    Some(Err(error)) => Some(AgentLoopError::llm(format!("Stream error: {error}"))),
                    None => None,
                };
                let continues = state.provider_finish_reason.as_deref() == Some("CONTINUATION");
                // Resume a dropped stream only at a checkpoint with no output
                // exposed after it. Replaying after a delta would duplicate it.
                let incomplete = continuation.is_some() && state.finish_reason.is_none();
                let resumes = incomplete
                    && !state.output_since_checkpoint
                    && state.continuation_token.is_some();
                if resumes && let Some((input, output, cached, thoughts)) = state.checkpoint_usage {
                    state.input_tokens = input;
                    state.output_tokens = output;
                    state.cached_tokens = cached;
                    state.thought_tokens = thoughts;
                }
                if let Some(context) = continuation.as_mut().filter(|_| continues || resumes) {
                    let spent = state
                        .billed_output
                        .saturating_add(state.output_tokens)
                        .saturating_add(state.thought_tokens.unwrap_or(0));
                    if context
                        .request
                        .generation_config
                        .as_ref()
                        .and_then(|cfg| cfg.max_output_tokens)
                        .is_some_and(|budget| spent >= budget)
                    {
                        state.finish_reason = Some("length".into());
                        state.finish();
                        continue;
                    }
                    let result = match state.continuation_token.take().filter(|t| !t.is_empty()) {
                        Some(token) => context.next(token).await,
                        None => Err(AgentLoopError::llm(
                            "Gemini CONTINUATION missing continuation token",
                        )),
                    };
                    match result {
                        Ok((bytes, retries)) => {
                            if retries.had_retries() {
                                state.retry_metadata.get_or_insert_default().absorb(retries);
                            }
                            state.next_hop();
                            stream = bytes.eventsource();
                        }
                        Err(error) => state.fail(error),
                    }
                } else if let Some(error) = error {
                    state.fail(error);
                } else if incomplete {
                    state.fail(AgentLoopError::llm("Gemini long decode stream ended before a safe continuation checkpoint or terminal response"));
                } else {
                    state.finish();
                }
            }
        },
    ))
}
