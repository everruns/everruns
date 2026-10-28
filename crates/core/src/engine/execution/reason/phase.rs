//! Reason admission, durable retry boundaries and terminal error handling.
use super::*;

impl ReasonAtom {
    pub(super) async fn execute_inner(
        &self,
        input: ReasonInput,
        assembled: Option<AssembledTurnContext>,
    ) -> Result<ReasonResult> {
        let operation = format!("{}/{}", input.context.turn_id, input.iteration);
        let retry_error = durable_retry::prepare(&operation).await?;
        // Keep the portable host's already large phase future off the caller's
        // stack, including ordinary native embedding with no retry host.
        Box::pin(self.execute_once(input, assembled, retry_error, &operation)).await
    }

    async fn execute_once(
        &self,
        input: ReasonInput,
        assembled: Option<AssembledTurnContext>,
        retry_error: Option<AgentLoopError>,
        retry_operation: &str,
    ) -> Result<ReasonResult> {
        let ReasonInput {
            context,
            harness_id,
            agent_id,
            org_id,
            mcp_tool_definitions,
            previous_response_id,
            iteration,
        } = input;

        tracing::info!(
            session_id = %context.session_id,
            turn_id = %context.turn_id,
            exec_id = %context.exec_id,
            harness_id = %harness_id,
            agent_id = ?agent_id,
            mcp_tools_count = %mcp_tool_definitions.len(),
            "ReasonAtom: starting LLM call"
        );

        // Generate OTel-style span IDs for hierarchical tracing
        // trace_id: groups all events in this turn
        // span_id: unique identifier for this reason span (shared by started/completed)
        // parent_span_id: links to turn as parent
        //
        // NOTE: TurnId::to_string() returns prefixed format (e.g., "turn_abc123")
        // matching the format used by turn.started/completed events in Braintrust.
        let trace_id = context.turn_id.to_string();
        let reason_span_id = Uuid::now_v7().to_string();
        let parent_span_id = trace_id.clone(); // Parent is the turn

        // Create event context from atom context with span info
        let event_context = EventContext::from_execution_context(&context).with_span(
            trace_id.clone(),
            reason_span_id.clone(),
            Some(parent_span_id.clone()),
        );

        let reason_start = Instant::now();

        // A lost attempt of this step already announced it (EVE-532).
        let prior = self.prior_stream(&context).await;
        if prior.announces_step()
            && let Err(e) = self
                .event_emitter
                .emit(
                    EventRequest::new(
                        context.session_id,
                        event_context.clone(),
                        ReasonStartedData {
                            harness_id,
                            agent_id,
                            metadata: None, // Will be populated after model resolution
                        },
                    )
                    .with_causal_context(&self.causal_context),
                )
                .await
        {
            tracing::warn!(
                session_id = %context.session_id,
                error = %e,
                "ReasonAtom: failed to emit reason.started event"
            );
        }

        // Assemble the turn context up-front so the error path below knows
        // the resolved provider/model and the error-disclosure mode even when
        // the LLM call (or the assembly itself) fails.
        let assembled = match (retry_error, assembled) {
            (Some(error), _) => Err(error),
            (None, Some(assembled)) => Ok(assembled),
            (None, None) => {
                self.context_resolver
                    .resolve_turn_context(TurnContextRequest {
                        session_id: context.session_id,
                        harness_id,
                        agent_id,
                        mcp_tool_definitions: mcp_tool_definitions.clone(),
                        allow_provider_managed_reduction: true,
                    })
                    .await
            }
        };
        let (error_disclosure, error_context, error_hooks, call_result) = match assembled {
            Ok(assembled) => {
                let (policy, retry, started) =
                    durable_retry::restore(retry_operation, &self.provider_retry_config).await?;
                // Heap-allocate the large fallback future so nested foreground
                // child turns fit Tokio's default worker stack.
                let call = durable_retry::attempt(Box::pin(
                    provider_managed_compaction::execute_with_fallback(
                        provider_managed_compaction::Call {
                            atom: self,
                            session_id: context.session_id,
                            harness_id,
                            agent_id,
                            org_id,
                            context: &context,
                            trace_id: &trace_id,
                            reason_span_id: &reason_span_id,
                            previous_response_id,
                            iteration,
                            mcp_tool_definitions: &mcp_tool_definitions,
                            assembled,
                            prior,
                        },
                    ),
                ));
                // Bound the complete recovered provider phase, including
                // compaction, on this instance's clock. Never poll a new call
                // when the authority's recovery budget has already expired.
                let exhausted = || provider_managed_compaction::CallOutcome {
                    disclosure: ErrorDisclosure::default(),
                    error_context: UserFacingErrorContext::default(),
                    error_hooks: Vec::new(),
                    result: Err(AgentLoopError::llm_kind(
                        everruns_contracts::error::LlmErrorKind::Unavailable,
                        "provider recovery time budget exhausted",
                    )
                    .with_retry_metadata(&retry)),
                };
                let outcome = match remaining_retry_time(&policy, started) {
                    Some(remaining) if remaining.is_zero() => exhausted(),
                    Some(remaining) => everruns_contracts::rt::timeout(remaining, call)
                        .await
                        .unwrap_or_else(|_| exhausted()),
                    None => call.await,
                };
                (
                    outcome.disclosure,
                    outcome.error_context,
                    outcome.error_hooks,
                    outcome.result,
                )
            }
            Err(error) => (
                ErrorDisclosure::default(),
                UserFacingErrorContext::default(),
                Vec::new(),
                Err(error),
            ),
        };

        let call_result = match call_result {
            Err(error) => match durable_retry::take_parking_error(error) {
                Ok(error) => return Err(error),
                Err(error) => Err(error),
            },
            result => result,
        };
        let call_result = match call_result {
            Err(error) if durable_retry::is_parked(&error) => return Err(error),
            Err(error) => match durable_retry::take_retryable_start(error) {
                Ok((error, policy)) => {
                    let error = durable_retry::retry(
                        retry_operation,
                        policy.as_ref().unwrap_or(&self.provider_retry_config),
                        error,
                    )
                    .await;
                    // Persistence failures and parking are host control flow.
                    if durable_retry::is_parked(&error) || !matches!(error, AgentLoopError::Llm(_))
                    {
                        return Err(error);
                    }
                    Err(error)
                }
                Err(error) => Err(error),
            },
            result => result,
        };

        // Handle LLM call errors gracefully
        let result = match call_result {
            Ok(result) => {
                // Calculate reason phase duration
                let reason_duration_ms = reason_start.elapsed().as_millis() as u64;

                // Emit reason.completed event (same span as reason.started, parent is turn)
                let completed_context = EventContext::from_execution_context(&context).with_span(
                    trace_id.clone(),
                    reason_span_id.clone(), // Same span_id as started
                    Some(parent_span_id.clone()),
                );
                if let Err(e) = self
                    .event_emitter
                    .emit(EventRequest::new(
                        context.session_id,
                        completed_context,
                        ReasonCompletedData::success(
                            &result.text,
                            result.has_tool_calls,
                            result.tool_calls.len() as u32,
                            Some(reason_duration_ms),
                            result.usage.clone(),
                        ),
                    ))
                    .await
                {
                    tracing::warn!(
                        session_id = %context.session_id,
                        error = %e,
                        "ReasonAtom: failed to emit reason.completed event"
                    );
                }
                result
            }
            // Not the model's failure: see `error_policy::returned_to_turn_driver`.
            Err(e) if error_policy::returned_to_turn_driver(&e, &context) => return Err(e),
            Err(e) => {
                let reason_duration_ms = reason_start.elapsed().as_millis() as u64;
                // LLM call failure is a "normal" result per the spec: report it.
                tracing::warn!(
                    session_id = %context.session_id,
                    turn_id = %context.turn_id,
                    error = %e,
                    "ReasonAtom: LLM call failed"
                );

                let error_msg = e.to_string();
                let mut source_error = e.user_facing_error(error_context);

                // Transient errors reach here only after in-call retries gave up (no
                // durable retry follows); they surface through `turn.failed` only.
                let is_transient = e.is_transient_llm_error()
                    || (e.llm_error_kind().is_none() && is_transient_error_message(&error_msg));

                // Capability error-hook seam: on the terminal (non-retried)
                // error path, let active capabilities react — perform a side
                // effect and/or augment the user-facing error fields — before the
                // message is built. The atom stays behavior-agnostic; each hook
                // (e.g. `usage_limit_auto_continue`) owns its own logic.
                let mut retry_schedule_id = None;
                if !error_hooks.is_empty() {
                    let services = crate::engine::llm_error_hook::LlmErrorHookServices {
                        schedule_store: self.schedule_store.clone(),
                    };
                    for (hook, config) in &error_hooks {
                        let outcome = {
                            let ctx = crate::engine::llm_error_hook::LlmErrorContext {
                                session_id: context.session_id,
                                error_code: &source_error.code,
                                error_fields: &source_error.fields,
                                config,
                                services: &services,
                            };
                            hook.on_llm_error(&ctx).await
                        };
                        retry_schedule_id = outcome.retry_schedule_id.or(retry_schedule_id);
                        for (key, value) in outcome.extra_error_fields {
                            source_error = source_error.with_field(key, value);
                        }
                    }
                }

                let user_error = source_error.apply_disclosure(error_disclosure, Some(&error_msg));
                let user_error_text = user_error.fallback_message();

                let mut output_message_id = None;

                if !is_transient {
                    // Create error message for the user to see
                    let mut error_message = RuntimeMessage::assistant(&user_error_text);
                    let mut metadata = std::collections::HashMap::new();
                    user_error.apply_to_message_metadata(&mut metadata);
                    UserFacingError::apply_disclosure_to_message_metadata(
                        &mut metadata,
                        error_disclosure,
                        &source_error.code,
                    );
                    error_message.metadata = Some(metadata);

                    output_message_id = Some(error_message.id);

                    // Emit output.message.completed event (stores message as event with proper turn context)
                    // output.message.completed is child of reason span
                    let error_msg_context = EventContext::from_execution_context(&context)
                        .with_span(
                            trace_id.clone(),
                            Uuid::now_v7().to_string(),   // Own span_id
                            Some(reason_span_id.clone()), // Parent is reason span
                        );
                    if let Err(emit_err) = self
                        .event_emitter
                        .emit(EventRequest::new(
                            context.session_id,
                            error_msg_context,
                            OutputMessageCompletedData::new(error_message)
                                .with_user_facing_error(&user_error)
                                .with_error_disclosure(error_disclosure),
                        ))
                        .await
                    {
                        tracing::warn!(
                            session_id = %context.session_id,
                            error = %emit_err,
                            "ReasonAtom: failed to emit output.message.completed event for error"
                        );
                    }
                } else {
                    tracing::info!(
                        session_id = %context.session_id,
                        "ReasonAtom: skipping error event for transient LLM error (will be retried)"
                    );
                }

                // Emit reason.completed event for failure (same span as started, parent is turn)
                let completed_context = EventContext::from_execution_context(&context).with_span(
                    trace_id.clone(),
                    reason_span_id.clone(), // Same span_id as started
                    Some(parent_span_id.clone()),
                );
                let mut failed_reason = EventRequest::new(
                    context.session_id,
                    completed_context,
                    ReasonCompletedData::failure(error_msg.clone(), Some(reason_duration_ms)),
                );
                if let Some(schedule_id) = retry_schedule_id {
                    // This trusted event context binds the retry to this exact input,
                    // even when generic disclosure removes user-facing error fields.
                    failed_reason = failed_reason.with_metadata(serde_json::json!({
                        "platform_auto_retry": { "schedule_id": schedule_id }
                    }));
                }
                if let Err(emit_err) = self.event_emitter.emit(failed_reason).await {
                    tracing::warn!(
                        session_id = %context.session_id,
                        error = %emit_err,
                        "ReasonAtom: failed to emit reason.completed event"
                    );
                }

                ReasonResult {
                    native_counts: None,
                    success: false,
                    text: user_error_text,
                    tool_calls: vec![],
                    has_tool_calls: false,
                    tool_definitions: vec![],
                    max_iterations: default_max_iterations(),
                    error: Some(error_msg.clone()),
                    user_facing_error: Some(user_error),
                    error_disclosure: Some(error_disclosure),
                    usage: None,
                    output_message_id,
                    time_to_first_token_ms: None,
                    response_id: None,
                    finish_reason: error_msg
                        .to_ascii_lowercase()
                        .contains("model refused")
                        .then(|| "refusal".to_string()),
                    ..ReasonResult::default()
                }
            }
        };

        Ok(result)
    }
}
