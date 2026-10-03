//! Everruns policy at the remote loop's boundaries (EVE-1124): parked calls,
//! policy stops, and the tool-result outbox. Split from the driver so the
//! durable orchestration and the policy decisions read separately.

use everruns_contracts::execution_phase::ExecutionPhase;
use everruns_contracts::tool_types::ToolCall;
use everruns_core::RuntimeMessage;
use everruns_core::agents_api_store::{
    ItemKind, ParkReason, PolicyStop, ReplacedMessage, ToolResultOutbox, ToolResultState,
};
use everruns_core::events::{
    EventRequest, ModelMetadata, OutputMessageCompletedData, OutputMessageReplacedData,
};
use serde_json::{Value, json};

use super::{
    AgentsApiTurnOutcome, FunctionBatch, FunctionCallAction, FunctionOutcome, MAX_CALL_ATTEMPTS,
    Recorded, Run, build_tool_result_input, store_error,
};
use crate::openai_agents_api::AgentsApiError;

impl Run<'_> {
    /// A previous Everruns turn that never reached an outcome (it was parked
    /// and then abandoned for a new message, or its activity failed) may
    /// still hold the provider turn open. Cancel it before this turn's input,
    /// so the provider does not keep waiting on a call nobody will answer.
    pub(super) async fn cancel_abandoned_turn(&self) {
        let Some(previous) = self.checkpoint.turn.as_ref() else {
            return;
        };
        if previous.turn_id == self.request.turn_id
            || previous.outcome.is_some()
            || previous.provider_turn_id.is_none()
        {
            return;
        }
        tracing::info!(
            session_id = %self.request.session_id,
            "Agents API: cancelling the unfinished provider turn of an earlier Everruns turn"
        );
        self.send_cancel().await;
    }

    async fn send_cancel(&self) {
        if let Some(session_id) = self.checkpoint.provider_session_id.as_deref()
            && let Err(error) = self
                .driver
                .client
                .send_events(
                    session_id,
                    vec![json!({"type": "agent.session.input.cancel"})],
                    None,
                )
                .await
        {
            tracing::warn!(%error, "Agents API cancel failed");
        }
    }

    /// Finish a saved policy stop: make sure a guardrail's replacement is in
    /// the log, stop the provider turn, and save the outcome.
    pub(super) async fn finish_policy_stop(
        &mut self,
    ) -> Result<AgentsApiTurnOutcome, AgentsApiError> {
        let stop = self
            .turn()
            .policy_stop
            .clone()
            .ok_or_else(|| AgentsApiError::Store("no policy stop".into()))?;
        self.close_runtime_policy_calls().await?;
        let outcome = match &stop.replaced {
            Some(replaced) => {
                let events = self.replacement_events(&stop, replaced);
                self.complete(
                    &replaced.item_key,
                    ItemKind::Message,
                    &replaced.message_id.to_string(),
                    Recorded::Message(replaced.message_id),
                    events,
                )
                .await?;
                AgentsApiTurnOutcome::Completed {
                    final_message_id: Some(replaced.message_id),
                    final_text: stop.message.clone(),
                    usage: None,
                    tool_calls: self.tool_call_count(),
                }
            }
            None => AgentsApiTurnOutcome::Failed {
                code: Some(stop.code.clone()),
                message: stop.message.clone(),
                policy: true,
            },
        };
        tracing::info!(
            session_id = %self.request.session_id,
            code = %stop.code,
            "Agents API: Everruns policy stopped the remote turn"
        );
        self.send_cancel().await;
        self.recover_policy_root().await?;
        // The stopped provider turn still spent tokens; bill them (or record
        // the amount as unknown) once.
        let final_text =
            matches!(outcome, AgentsApiTurnOutcome::Completed { .. }).then(|| stop.message.clone());
        let usage = self.account(None, final_text, true).await?;
        let mut outcome = outcome;
        if let AgentsApiTurnOutcome::Completed {
            usage: reported, ..
        } = &mut outcome
        {
            *reported = usage;
        }
        self.turn_mut().outcome = Some(serde_json::to_value(&outcome).map_err(store_error)?);
        self.save().await?;
        self.release().await?;
        Ok(outcome)
    }

    /// `output.message.replaced`, then the replacement as the canonical
    /// message. The withheld provider text is never recorded.
    fn replacement_events(
        &self,
        stop: &PolicyStop,
        replaced: &ReplacedMessage,
    ) -> Vec<EventRequest> {
        let message = RuntimeMessage::assistant(stop.message.clone())
            .with_id(replaced.message_id)
            .with_phase(ExecutionPhase::FinalAnswer);
        vec![
            self.event(
                Some(&replaced.provider_item_id),
                OutputMessageReplacedData {
                    turn_id: self.request.turn_id,
                    message_id: replaced.message_id,
                    guardrail_capability_id: replaced.guardrail_capability_id.clone(),
                    guardrail_id: replaced.guardrail_id.clone(),
                    reason_code: replaced.reason_code.clone(),
                    replacement: stop.message.clone(),
                },
            ),
            self.event(
                Some(&replaced.provider_item_id),
                OutputMessageCompletedData::new(message).with_metadata(ModelMetadata {
                    model: self.model(),
                    model_id: None,
                    provider_id: None,
                }),
            ),
        ]
    }

    pub(super) fn tool_call_count(&self) -> u32 {
        let count = self
            .turn()
            .items
            .keys()
            .filter_map(|key| {
                if key.starts_with("call:") {
                    Some(key.as_str())
                } else {
                    // A pre-upgrade MCP start and its hosted lifecycle refer
                    // to the same provider item, so count that item once.
                    key.strip_prefix("mcp:")
                        .or_else(|| key.strip_prefix("hosted:"))
                }
            })
            .collect::<std::collections::HashSet<_>>()
            .len();
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    /// Answer pending client function calls of this turn through the
    /// tool-result outbox, applying Everruns policy at the boundary: every
    /// call runs through the tool pipeline, a call the pipeline parks keeps
    /// the provider's required action open, and a policy halt stops the turn.
    pub(super) async fn handle_actions(
        &mut self,
        actions: Vec<FunctionCallAction>,
    ) -> Result<(), AgentsApiError> {
        let Some(own_turn) = self.turn().provider_turn_id.clone() else {
            return Ok(());
        };
        if self.turn().policy_stop.is_some() {
            return Ok(());
        }
        let actions: Vec<_> = actions
            .into_iter()
            .filter(|action| action.turn_id == own_turn)
            .collect();
        let mut to_run = Vec::new();
        for action in &actions {
            self.record_function_call(&action.call_id, &action.name, &action.arguments)
                .await?;
            let entry = self.turn().tool_results.get(&action.call_id).cloned();
            let Some(entry) = entry else {
                self.turn_mut().tool_results.insert(
                    action.call_id.clone(),
                    ToolResultOutbox {
                        provider_turn_id: own_turn.clone(),
                        call_id: action.call_id.clone(),
                        name: action.name.clone(),
                        arguments: action.arguments.clone(),
                        attempt: 0,
                        state: ToolResultState::Executing,
                    },
                );
                to_run.push(action.call_id.clone());
                continue;
            };
            match entry.state.clone() {
                ToolResultState::Submitted { .. } | ToolResultState::Ready { .. } => {}
                ToolResultState::Executing => {
                    // A previous owner claimed the call. Reuse its recorded
                    // result; only an unrecorded call re-enters the pipeline,
                    // whose durable claim decides whether it may run again.
                    match self
                        .driver
                        .ledger
                        .tool_result(self.request.session_id, &entry.local_call_id())
                        .await?
                    {
                        Some(result) => self.set_ready(&action.call_id, result),
                        None => to_run.push(action.call_id.clone()),
                    }
                }
                ToolResultState::Parked { reason, iteration } => {
                    // THREAT[TM-TOOL-051]: a replay of the activity that
                    // parked stays parked; only the resumed turn resolves it.
                    if self.request.iteration <= iteration {
                        self.paused = true;
                        continue;
                    }
                    self.resolve_parked(&entry, reason, &mut to_run).await?;
                }
            }
        }
        self.save().await?;
        if !to_run.is_empty() {
            self.run_calls(&to_run).await?;
            if self.turn().policy_stop.is_some() {
                return Ok(());
            }
        }
        for action in &actions {
            self.submit_ready(&own_turn, &action.call_id).await?;
        }
        Ok(())
    }

    /// The turn resumed after a parked call's pause was answered.
    async fn resolve_parked(
        &mut self,
        entry: &ToolResultOutbox,
        reason: ParkReason,
        to_run: &mut Vec<String>,
    ) -> Result<(), AgentsApiError> {
        let session_id = self.request.session_id;
        match reason {
            ParkReason::ClientResult => {
                let result = self
                    .driver
                    .ledger
                    .tool_result(session_id, &entry.local_call_id())
                    .await?
                    .unwrap_or_else(|| {
                        Err("The client did not return a result for this tool call.".to_string())
                    });
                self.set_ready(&entry.call_id, result);
            }
            ParkReason::Approval { request_call_id } => {
                let decision = self
                    .driver
                    .ledger
                    .tool_result(session_id, &request_call_id)
                    .await?;
                match approval_decision(decision.as_ref()) {
                    // THREAT[TM-TOOL-008]: only a recorded approval runs the
                    // call again, and the gate still checks it on that run.
                    Ok(()) => self.retry_call(entry, to_run).await?,
                    Err(outcome) => self.set_ready(
                        &entry.call_id,
                        Err(format!(
                            "The tool call was not approved ({outcome}), so it did not run."
                        )),
                    ),
                }
            }
            ParkReason::Retry => self.retry_call(entry, to_run).await?,
        }
        Ok(())
    }

    /// Run a parked call again under a fresh local id, recorded as a new
    /// assistant tool call the way a native model retries after a pause.
    async fn retry_call(
        &mut self,
        entry: &ToolResultOutbox,
        to_run: &mut Vec<String>,
    ) -> Result<(), AgentsApiError> {
        let attempt = entry.attempt + 1;
        if attempt >= MAX_CALL_ATTEMPTS {
            self.set_ready(
                &entry.call_id,
                Err("The tool call still needs a user action, so it did not run.".to_string()),
            );
            return Ok(());
        }
        if let Some(outbox) = self.turn_mut().tool_results.get_mut(&entry.call_id) {
            outbox.attempt = attempt;
            outbox.state = ToolResultState::Executing;
        }
        self.save().await?;
        let local = everruns_core::agents_api_store::local_call_id(&entry.call_id, attempt);
        self.record_function_call(&local, &entry.name, &entry.arguments)
            .await?;
        to_run.push(entry.call_id.clone());
        Ok(())
    }

    /// Run claimed calls through the tool pipeline as one batch, so their
    /// pauses land in one request a person answers at once.
    async fn run_calls(&mut self, call_ids: &[String]) -> Result<(), AgentsApiError> {
        let calls: Vec<ToolCall> = call_ids
            .iter()
            .filter_map(|call_id| self.turn().tool_results.get(call_id))
            .map(|entry| ToolCall {
                id: entry.local_call_id(),
                name: entry.name.clone(),
                arguments: entry.arguments.clone(),
            })
            .collect();
        match self.driver.executor.execute(&calls).await? {
            FunctionBatch::Halt { code, message } => {
                self.turn_mut().policy_stop = Some(PolicyStop {
                    code,
                    message,
                    replaced: None,
                });
            }
            FunctionBatch::Outcomes(outcomes) => {
                if outcomes.len() != call_ids.len() {
                    return Err(AgentsApiError::Store(format!(
                        "tool pipeline returned {} outcomes for {} calls",
                        outcomes.len(),
                        call_ids.len()
                    )));
                }
                let iteration = self.request.iteration;
                for (call_id, outcome) in call_ids.iter().zip(outcomes) {
                    match outcome {
                        FunctionOutcome::Done(result) => self.set_ready(call_id, result),
                        FunctionOutcome::Parked(reason) => {
                            self.paused = true;
                            if let Some(entry) = self.turn_mut().tool_results.get_mut(call_id) {
                                entry.state = ToolResultState::Parked { reason, iteration };
                            }
                        }
                    }
                }
            }
        }
        self.save().await
    }

    fn set_ready(&mut self, call_id: &str, result: Result<String, String>) {
        let (success, output) = match result {
            Ok(output) => (true, output),
            Err(error) => (false, error),
        };
        if let Some(entry) = self.turn_mut().tool_results.get_mut(call_id) {
            entry.state = ToolResultState::Ready { success, output };
        }
    }

    /// Submit a ready result with its idempotency key.
    async fn submit_ready(&mut self, own_turn: &str, call_id: &str) -> Result<(), AgentsApiError> {
        let Some(ToolResultState::Ready { success, output }) = self
            .turn()
            .tool_results
            .get(call_id)
            .map(|entry| entry.state.clone())
        else {
            return Ok(());
        };
        let session_id = self.provider_session()?;
        let result = if success {
            Ok(output.as_str())
        } else {
            Err(output.as_str())
        };
        let key = format!(
            "everruns-tool-result-{}-{}",
            self.request.session_id, call_id
        );
        match self
            .driver
            .client
            .send_events(
                &session_id,
                vec![build_tool_result_input(own_turn, call_id, result)],
                Some(&key),
            )
            .await
        {
            Ok(()) => {}
            // The provider no longer accepts a result for this call: it
            // already has one or the turn ended. Saved items tell which.
            Err(error) if error.is_conflict() => {
                tracing::info!(%call_id, %error, "Agents API refused a repeated tool result");
            }
            Err(error) => return Err(error),
        }
        if let Some(entry) = self.turn_mut().tool_results.get_mut(call_id) {
            entry.state = ToolResultState::Submitted {
                success,
                output: output.clone(),
            };
        }
        self.save().await
    }
}

/// Whether a recorded `approve_tool_call` result approved the call. `Err`
/// carries how it ended otherwise; no recorded decision is not an approval.
fn approval_decision(recorded: Option<&Result<String, String>>) -> Result<(), String> {
    let Some(Ok(text)) = recorded else {
        return Err("no decision".to_string());
    };
    let summary: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    if summary.get("approved").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    Err(summary
        .get("outcome")
        .and_then(Value::as_str)
        .unwrap_or("not approved")
        .to_string())
}
