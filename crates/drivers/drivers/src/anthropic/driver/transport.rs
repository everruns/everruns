//! Messages HTTP transport, bounded retries and optional thread replay.
use super::*;

impl AnthropicChatDriver {
    /// Send one streaming Messages request, applying the shared header-phase
    /// retry loop (transient send failures, 429, 5xx, and the one-shot
    /// `max_tokens` fallback), and return the raw response plus its retry
    /// metadata.
    ///
    /// Invoked once per reconnect attempt by [`connect_sse_with_reconnect`]. The
    /// `request` is shared (`Arc<Mutex>`) so a fallback-corrected body carries
    /// across reconnects. Thread continuations rejected after an uncertain
    /// attempt replay from full history; other errors keep their classification.
    pub(super) async fn send_messages_request(
        &self,
        endpoint: &everruns_contracts::ProviderEndpoint,
        request: Arc<Mutex<AnthropicRequest>>,
        options: SendMessagesOptions<'_>,
        retries_consumed: u32,
        thread: &Arc<Mutex<message_threads::Exchange>>,
    ) -> Result<(reqwest::Response, RetryMetadata)> {
        let SendMessagesOptions {
            needs_interleaved_thinking,
            wants_million_context,
            wants_cache_diagnostics,
            conversation_betas,
            wants_server_compaction,
            max_tokens_from_profile,
            model,
            extra_headers,
        } = options;
        let last_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let max_tokens_fallback_attempted = Arc::new(Mutex::new(false));
        let beta_logged = Arc::new(Mutex::new(false));
        let binds_thinking = layout::binds_thinking_to_conversation(model);
        let mut retry_config = self.retry_config.clone();
        retry_config.max_retries = retry_config.max_retries.saturating_sub(retries_consumed);

        retry_request(
            &retry_config,
            "AnthropicDriver",
            || {
                let request = Arc::clone(&request);
                let beta_logged = Arc::clone(&beta_logged);
                async move {
                    // Build request with headers (must rebuild each iteration).
                    let url = endpoint.url("messages").ok_or_else(|| {
                        SendOutcome::Fatal(AgentLoopError::config("Anthropic provider has no base URL"))
                    })?;
                    let mut driver_headers: Vec<(String, String)> = vec![
                        ("anthropic-version".to_string(), ANTHROPIC_VERSION.to_string()),
                        ("Content-Type".to_string(), "application/json".to_string()),
                    ];

                    // Anthropic beta features, combined into a single
                    // comma-separated `anthropic-beta` header:
                    //  - interleaved-thinking: required for tool use with
                    //    budget-based extended thinking (adaptive thinking
                    //    interleaves on its own).
                    //  - context-1m: opts the `[1m]` model ids into the 1M
                    //    context window. It is GA / silently ignored on Opus 4.6+
                    //    and Fable 5.x (the only ids we attach it to), so always
                    //    sending it is safe.
                    //  - cache-diagnosis: required for the request-level
                    //    `diagnostics` object and the diagnostics payload on
                    //    the response.
                    let mut beta_features: Vec<&str> = Vec::new();
                    if needs_interleaved_thinking {
                        beta_features.push("interleaved-thinking-2025-05-14");
                    }
                    if wants_million_context {
                        beta_features.push("context-1m-2025-08-07");
                    }
                    if wants_cache_diagnostics {
                        beta_features.push(CACHE_DIAGNOSTICS_BETA);
                    }
                    if binds_thinking {
                        beta_features.push(layout::THINKING_BINDING_BETA);
                    }
                    beta_features.extend(conversation_betas);
                    if wants_server_compaction {
                        beta_features.push(SERVER_COMPACTION_BETA);
                    }
                    if !beta_features.is_empty() {
                        let beta = beta_features.join(",");
                        let mut logged = beta_logged.lock().unwrap();
                        if !*logged {
                            tracing::info!(beta = %beta, "AnthropicDriver: enabling anthropic-beta features");
                            *logged = true;
                        }
                        drop(logged);
                        driver_headers.push(("anthropic-beta".to_string(), beta));
                    }

                    // Snapshot the (possibly fallback-mutated) request as JSON
                    // while holding the lock, then release it before awaiting the
                    // send so the guard never crosses an `.await` point.
                    let full_body = serde_json::to_vec(&*request.lock().unwrap())
                        .map_err(|e| SendOutcome::Fatal(AgentLoopError::llm(format!(
                            "Failed to serialize Anthropic request: {e}"
                        ))))?;
                    let resolved = endpoint.resolve("POST", url.clone(), &full_body)
                        .await.map_err(SendOutcome::Fatal)?;
                    driver_headers.extend(resolved.headers.clone());
                    let mut headers = driver_helpers::merge_request_headers(driver_headers, extra_headers);
                    let (body, threaded) = {
                        let mut exchange = thread.lock().unwrap();
                        if exchange.enabled() {
                            message_threads::add_beta(&mut headers);
                            let full = serde_json::from_slice(&full_body).map_err(|e|
                                SendOutcome::Fatal(AgentLoopError::llm(format!("Invalid serialized request: {e}"))))?;
                            let body = exchange.prepare(full, &resolved.url, &headers);
                            (serde_json::to_vec(&body).map_err(|e|
                                SendOutcome::Fatal(AgentLoopError::llm(e.to_string())))?, true)
                        } else {
                            // Keep the default path's serialization and allocation
                            // behavior, including after an unsupported-thread replay.
                            exchange.sent_body = Value::Null;
                            (full_body, false)
                        }
                    };
                    // Authentication can sign the payload. Resolve again only
                    // when the thread changed it, with the actual bytes sent.
                    let resolved = if threaded {
                        let resolved = endpoint.resolve("POST", url, &body)
                            .await.map_err(SendOutcome::Fatal)?;
                        headers = driver_helpers::merge_request_headers(headers, &resolved.headers);
                        headers = driver_helpers::merge_request_headers(headers, extra_headers);
                        if thread.lock().unwrap().enabled() {
                            message_threads::add_beta(&mut headers);
                        }
                        resolved
                    } else {
                        resolved
                    };
                    let mut request_builder = self.client().post(&resolved.url);
                    for (name, value) in headers {
                        request_builder = request_builder.header(name, value);
                    }
                    request_builder
                        .body(body)
                        .send()
                        .await
                        .map_err(SendOutcome::Send)
                }
            },
            |response, attempts, can_retry| {
                let request = Arc::clone(&request);
                let last_error = Arc::clone(&last_error);
                let max_tokens_fallback_attempted = Arc::clone(&max_tokens_fallback_attempted);
                let model = model.to_string();
                async move {
                    let status = response.status();

                    if can_retry {
                        // Parse rate limit info from headers before consuming body.
                        let rate_limit_info = if is_rate_limit_status(status) {
                            Some(RateLimitInfo::from_anthropic_headers(response.headers()))
                        } else {
                            None
                        };

                        let error_text = response.text().await.unwrap_or_default();

                        // Don't retry a request-too-large error (not transient).
                        if is_anthropic_request_too_large(status, &error_text) {
                            return RetryDecision::Terminal(AgentLoopError::request_too_large(
                                format!("Anthropic API error ({}): {}", status, error_text),
                            ));
                        }

                        // Exhausted billing quota is not transient — fail fast.
                        if is_provider_quota_message(&error_text) {
                            return RetryDecision::Terminal(AgentLoopError::llm_kind(
                                LlmErrorKind::QuotaExhausted,
                                format!("Anthropic API error ({}): {}", status, error_text),
                            ));
                        }

                        let wait = rate_limit_info
                            .as_ref()
                            .map(|info| info.recommended_wait(&self.retry_config, attempts))
                            .unwrap_or_else(|| self.retry_config.calculate_backoff(attempts));

                        *last_error.lock().unwrap() = Some(error_text);
                        return RetryDecision::Retry {
                            wait,
                            rate_limit_info,
                        };
                    }

                    // Non-retryable error or max retries exceeded
                    let error_text = response.text().await.unwrap_or_default();
                    let error_msg = format!("Anthropic API error ({}): {}", status, error_text);
                    if thread.lock().unwrap().recover(status.as_u16(), &error_text) {
                        return RetryDecision::RetryNow;
                    }

                    // Graceful fallback: if max_tokens derived from profile
                    // exceeds the model limit (400 error), retry once with a safe
                    // fallback value instead of failing immediately. Modeled as
                    // `RetryNow` so it re-sends without counting an attempt or
                    // sleeping, matching the original `continue`.
                    {
                        let mut fallback_attempted = max_tokens_fallback_attempted.lock().unwrap();
                        if status == reqwest::StatusCode::BAD_REQUEST
                            && !*fallback_attempted
                            && max_tokens_from_profile
                            && (error_text.contains("max_tokens")
                                || error_text.contains("maximum output tokens"))
                        {
                            const FALLBACK_MAX_TOKENS: u32 = 16_384;
                            let mut req = request.lock().unwrap();
                            tracing::warn!(
                                attempted = req.max_tokens,
                                fallback = FALLBACK_MAX_TOKENS,
                                model = %model,
                                "max_tokens exceeds model limit, retrying with fallback. \
                                 Update model profile for {}.",
                                model,
                            );
                            req.max_tokens = FALLBACK_MAX_TOKENS;
                            *fallback_attempted = true;
                            return RetryDecision::RetryNow;
                        }
                    }

                    // Check if this is a model-not-found error (404 with not_found_error)
                    if is_anthropic_model_not_found(status, &error_text) {
                        return RetryDecision::Terminal(AgentLoopError::model_not_available(model));
                    }

                    // Check if this is a request-too-large error
                    if is_anthropic_request_too_large(status, &error_text) {
                        return RetryDecision::Terminal(AgentLoopError::request_too_large(error_msg));
                    }

                    if wants_server_compaction
                        && server_compaction::is_rejection_response(status, &error_text)
                        && server_compaction::record_rejection(endpoint, &model)
                    {
                        tracing::warn!(
                            model = %model,
                            status = status.as_u16(),
                            "AnthropicDriver: server compaction rejected; caching legacy fallback"
                        );
                        return RetryDecision::Terminal(
                            AgentLoopError::provider_capability_rejected(
                                RejectedProviderCapability::AnthropicServerCompaction,
                                status.as_u16(),
                                &error_text,
                                error_msg,
                            ),
                        );
                    }

                    // Classified, and its status preserved, while the HTTP
                    // response is still structured (see LlmError).
                    let message = if attempts > 0 {
                        format!(
                            "{} (after {} retries, last error: {})",
                            error_msg,
                            attempts,
                            last_error.lock().unwrap().take().unwrap_or_default()
                        )
                    } else {
                        error_msg
                    };
                    RetryDecision::Terminal(AgentLoopError::llm_http(
                        status.as_u16(),
                        &error_text,
                        message,
                    ))
                }
            },
            |e, attempts| AgentLoopError::llm(send_error_message(e, attempts)),
        )
        .await
    }
}
