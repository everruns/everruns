//! Framework lifecycle around the shared actor driver, under its single epoch.
use super::*;
use crate::{
    Agent, Engine, Harness, Turn, TurnStopReason,
    hooks::{AgentStartContext, CompletionContext, HookRunState, TurnStartContext},
};
use everruns_contracts::{error::AgentLoopError, typed_id::TurnId};
use everruns_core::{
    events::{Event, EventData},
    host::{
        InProcessRuntime, TurnDrive,
        session_actor::{
            ActorHosts, HostTurn, InputPreparation, SessionActorConfig, SessionActorDriver,
            TurnSignals,
        },
    },
};
use serde_json::json;

pub(crate) async fn run(
    engine: &Engine,
    session: SessionId,
    agent: Agent,
    harness: Option<Harness>,
    activation: SessionActivation,
    config: SessionActorConfig,
) -> Result<()> {
    let epoch = activation.epoch;
    expire_waits(engine, session, epoch.as_ref()).await?;
    let rejected = epoch
        .records("rejected/")
        .await?
        .into_iter()
        .map(|(key, _)| {
            key.strip_prefix("rejected/")
                .unwrap_or("")
                .parse::<MessageId>()
                .map_err(|_| AgentLoopError::store("invalid input rejection identity"))
        })
        .collect::<Result<std::collections::HashSet<_>>>()?;
    let mut backends = epoch.backends();
    backends.event_log = Arc::new(super::query::InputFilterLog::new(
        backends.event_log,
        rejected,
    ));
    let state = HookRunState::durable(agent.lifecycle_hooks(), epoch.clone());
    let handle = engine.resume_hosted(session).await?;
    let environment = if activation.binding.environment.is_some() {
        engine.hosted_environment(session).await?
    } else {
        agent
            .default_session_environment(session)
            .await
            .map_err(|error| AgentLoopError::config(error.to_string()))?
    };
    if let Some(harness) = &harness {
        harness
            .negotiate(&environment)
            .map_err(|e| AgentLoopError::config(e.to_string()))?;
    }
    let live = engine.hosted_live_controls(session).await?;
    let runtime = match everruns_contracts::rt::timeout(
        config.lease_ttl,
        agent.build_runtime_with_event_sink(
            backends,
            session,
            Some(environment),
            harness.as_ref(),
            handle.inner.event_bus.clone(),
            state.clone(),
            Some(live.as_ref()),
        ),
    )
    .await
    {
        Ok(Ok(runtime)) => runtime,
        Ok(Err(error)) => {
            if error.is_non_retryable() {
                epoch.suspend("runtime_configuration_error").await?;
            }
            return Err(error);
        }
        Err(_) => {
            return Err(AgentLoopError::store(
                "Engine runtime construction timed out",
            ));
        }
    };
    let store = epoch.actor_store();
    let renew_poke = epoch.renew_poke();
    let hosts = Arc::new(FrameworkHosts {
        engine: engine.clone(),
        runtime,
        epoch,
        agent,
        state,
        session,
    });
    let mut driver = SessionActorDriver::new(hosts, store, config);
    if let Some(poke) = renew_poke {
        driver = driver.with_renew_poke(poke);
    }
    driver.run_actor(activation.actor).await;
    Ok(())
}

struct FrameworkHosts {
    engine: Engine,
    runtime: InProcessRuntime,
    epoch: Arc<dyn SessionEpoch>,
    agent: Agent,
    state: Arc<HookRunState>,
    session: SessionId,
}

#[async_trait]
impl ActorHosts for FrameworkHosts {
    type Host = InProcessRuntime;
    fn host(&self, _: HostTurn, signals: &TurnSignals) -> Self::Host {
        if let Some(fence) = &signals.fence {
            self.epoch.observe_fence(fence);
        }
        // Capability code the host registered on the Engine (EVE-1297)
        // resolves from the next turn of a running activation.
        for capability in self.engine.registered_capabilities() {
            if let Err(error) =
                crate::session::register_engine_capability(&self.runtime, &capability)
            {
                tracing::warn!(%error, "Engine capability not registered on activation runtime");
            }
        }
        self.runtime.clone()
    }

    async fn fresh_turn_id(&self, input: MessageId, proposed: TurnId) -> Result<TurnId> {
        let key = format!("turn_for/{input}");
        self.epoch.commit(&key, json!(proposed), true).await?;
        serde_json::from_value(
            self.epoch
                .record(&key)
                .await?
                .ok_or_else(|| AgentLoopError::store("missing retained turn identity"))?,
        )
        .map_err(|_| AgentLoopError::store("invalid retained turn identity"))
    }

    async fn inputs_consumed(&self, drive: &TurnDrive, inputs: &[MessageId]) -> Result<()> {
        for input in inputs {
            let key = format!("assignment/{input}");
            let value =
                json!({"turn_id": drive.turn_id, "steered": *input != drive.input_message_id});
            if !self.epoch.commit(&key, value.clone(), true).await?
                && self.epoch.record(&key).await? != Some(value)
            {
                return Err(AgentLoopError::store(
                    "admission already belongs to another turn",
                ));
            }
        }
        Ok(())
    }

    async fn before_turn(
        &self,
        _: &Self::Host,
        drive: &TurnDrive,
        input: Option<&RuntimeMessage>,
    ) -> Result<InputPreparation> {
        let input = input
            .ok_or_else(|| AgentLoopError::store("admitted input missing from resolved history"))?;
        let result = async {
            self.state
                .durable_agent_start(
                    drive.input_message_id,
                    AgentStartContext {
                        agent_name: self.agent.name().to_owned(),
                        session_id: self.session,
                    },
                )
                .await?;
            self.state
                .durable_turn_start(
                    &drive.turn_id.to_string(),
                    TurnStartContext {
                        agent_name: self.agent.name().to_owned(),
                        session_id: self.session,
                        input: crate::InputMessage::from_message(input),
                    },
                )
                .await
        }
        .await;
        if let Err(failure) = result {
            self.epoch
                .commit(
                    &format!("gate_failure/{}", drive.turn_id),
                    json!(failure),
                    false,
                )
                .await?;
            if self
                .state
                .gate_failure_is_known(drive.input_message_id, &drive.turn_id.to_string(), &failure)
                .await?
            {
                self.epoch
                    .commit(
                        &format!("rejected/{}", drive.input_message_id),
                        json!({"turn_id":drive.turn_id,"failure":failure}),
                        true,
                    )
                    .await?;
                return Ok(InputPreparation::Rejected);
            }
            self.epoch.suspend("lifecycle_gate_failed").await?;
            return Err(AgentLoopError::config("Framework lifecycle gate failed"));
        }
        Ok(InputPreparation::Ready)
    }

    async fn checkpoint(&self, _: &Self::Host, tail: &[Event]) -> Result<()> {
        let Some(mut turn) = project_terminal(tail) else {
            let pending = self.engine.session_status(self.session).await?.waiting;
            self.epoch
                .schedule(pending.iter().filter_map(approval_deadline).min())
                .await?;
            return Ok(());
        };
        self.epoch.schedule(None).await?;
        let key = format!("outcome/{}", turn.turn_id);
        if self.epoch.record(&key).await?.is_some() {
            return Ok(());
        }
        turn.hook_failures.extend(self.state.take_failures());
        for (_, record) in self
            .epoch
            .records(&format!("failure/{}/", turn.turn_id))
            .await?
        {
            let failure: crate::HookFailure = serde_json::from_value(record)
                .map_err(|_| AgentLoopError::store("invalid lifecycle failure receipt"))?;
            if !turn.hook_failures.contains(&failure) {
                turn.hook_failures.push(failure);
            }
        }
        for failure in self.state.recovered_tool_failures(&turn.turn_id).await? {
            if !turn.hook_failures.iter().any(|known| {
                known.point == failure.point
                    && known.handler_index == failure.handler_index
                    && known.tool_call_id == failure.tool_call_id
            }) {
                turn.hook_failures.push(failure);
            }
        }
        if turn.stop_reason != TurnStopReason::Cancelled {
            let context = CompletionContext {
                agent_name: self.agent.name().to_owned(),
                session_id: self.session,
                turn: turn.clone(),
            };
            turn.hook_failures
                .extend(self.state.durable_completion(context).await);
        }
        self.epoch.commit(&key, json!(turn), true).await?;
        Ok(())
    }
}

/// Runtime termination is projected separately from Framework finalization.
fn project_terminal(events: &[Event]) -> Option<Turn> {
    let terminal = events.iter().rev().find(|event| {
        matches!(
            event.data,
            EventData::TurnCompleted(_)
                | EventData::TurnFailed(_)
                | EventData::TurnCancelled(_)
                | EventData::TurnSealed(_)
        )
    })?;
    let turn_id = terminal.context.turn_id?;
    let events = events
        .iter()
        .filter(|event| event.context.turn_id == Some(turn_id));
    let mut turn = Turn {
        response: String::new(),
        turn_id: turn_id.to_string(),
        stop_reason: TurnStopReason::Error,
        iterations: 0,
        tool_calls: 0,
        success: false,
        error: None,
        hook_failures: Vec::new(),
    };
    let mut said = Vec::new();
    for event in events {
        match &event.data {
            EventData::OutputMessageCompleted(data) => {
                if let Some(message) =
                    everruns_core::conversation::SaidMessage::from_message(&data.message)
                {
                    said.push(message);
                }
            }
            EventData::ReasonStarted(_) => turn.iterations += 1,
            EventData::ToolCompleted(_) => turn.tool_calls += 1,
            EventData::TurnCompleted(data) => {
                turn.iterations = data.iterations as usize;
                turn.stop_reason = data.termination_reason.unwrap_or_else(|| {
                    TurnStopReason::from_provider_finish_reason(data.stop_reason.as_deref())
                });
                turn.success = true;
            }
            EventData::TurnFailed(data) => {
                turn.stop_reason = TurnStopReason::Error;
                turn.error = Some(data.error.clone());
            }
            EventData::TurnCancelled(_) => {
                turn.stop_reason = TurnStopReason::Cancelled;
                turn.error = Some("turn cancelled".into());
            }
            EventData::TurnSealed(data) => {
                turn.stop_reason = TurnStopReason::Error;
                turn.error = Some(data.reason.clone());
            }
            _ => {}
        }
    }
    if turn.success {
        turn.response = everruns_core::conversation::final_reply(said).unwrap_or_default();
    }
    Some(turn)
}

fn approval_deadline(call: &crate::ToolCall) -> Option<i64> {
    let request = everruns_contracts::tool_approval_types::ToolApprovalRequired::from_request_call(
        &call.id,
        &call.name,
        &call.arguments,
    )?;
    request
        .expires_at
        .parse::<everruns_contracts::rt::UtcTimestamp>()
        .ok()
        .map(|at| at.timestamp_millis())
}

// Timers only request an activation. The shared Framework execution path
// interprets the deadline and authors a fenced failure, never a permission.
async fn expire_waits(engine: &Engine, session: SessionId, epoch: &dyn SessionEpoch) -> Result<()> {
    let status = engine.session_status(session).await?;
    let expired: Vec<_> = status
        .waiting
        .iter()
        .filter(|call| approval_deadline(call).is_some_and(|at| at <= status.now_ms))
        .collect();
    if expired.is_empty() {
        return Ok(());
    }
    let mut contexts = std::collections::HashMap::new();
    let store = epoch.actor_store();
    let mut after = None;
    loop {
        let page = store.read_actor_journal(session, after, 256).await?;
        for event in &page.events {
            if let EventData::ToolCallRequested(data) = &event.data {
                for call in &data.tool_calls {
                    contexts.insert(call.id.clone(), event.context.clone());
                }
            }
        }
        if !page.has_more {
            break;
        }
        after = page.events.last().and_then(|event| event.sequence);
        if after.is_none() {
            return Err(AgentLoopError::store("invalid journal continuation"));
        }
    }
    for call in expired {
        let context = contexts
            .get(&call.id)
            .ok_or_else(|| AgentLoopError::store("pending request has no canonical context"))?;
        epoch.backends().event_log.append(everruns_core::events::EventRequest::new(session,context.clone(),ToolCompletedData::failure(call.id.clone(),call.name.clone(),"timeout".into(),"Approval request expired without permission. Ask for a new decision before performing this action.".into(),None))).await.map_err(|error|AgentLoopError::store(error.to_string()))?;
    }
    Ok(())
}
