//! Volatile authority. Control, admission, journal writes and receipts share
//! one per-session gate; epochs are checked while that gate is held.
use super::*;
use everruns_contracts::{error::AgentLoopError, rt::Instant, typed_id::TurnId};
use everruns_core::{
    CompactionCheckpoint, CompactionCheckpointStore, ExecutionSession, ProactiveCompactionAttempt,
    events::{Event, EventContext, EventData, EventRequest, InputMessageData},
    execution_loading::SessionStore,
    host::{
        EventCursor, EventDurability, EventLog, EventLogError, EventPage, EventReadLimit,
        EventReadRequest, EventReader, InMemoryEventLog, RuntimeSessionStore, SessionMutator,
        session_actor::store::{ActorJournal, ActorRelease, ActorRenewal, actor_lease_lost},
    },
    session_services::{KeyInfo, SecretInfo, SessionStorageStore},
};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, RwLock};

/// A coherent process-lifetime authority. Clones share sessions. There is no
/// claim that accepted work survives losing this process or Wasm isolate.
#[derive(Clone, Default)]
pub struct MemoryHosting {
    sessions: Arc<Mutex<HashMap<SessionId, Arc<MemorySession>>>>,
}

struct MemorySession {
    id: SessionId,
    renew_poke: Arc<tokio::sync::Notify>,
    control: tokio::sync::Mutex<Control>,
    log: InMemoryEventLog,
    services: HostBackends,
}

#[derive(Default)]
struct Control {
    wake_at: Option<i64>,
    created_at: String,
    binding: Option<SessionBinding>,
    inbox: i64,
    consumed: i64,
    next_input: i64,
    holder: Option<String>,
    fence: u64,
    expires: Option<Instant>,
    attempts: u32,
    cancel: Option<i64>,
    cancel_reported: bool,
    resume_generation: u64,
    resume_claimed: u64,
    runnable: bool,
    suspended: bool,
    open: Option<TurnId>,
    pending: Vec<crate::ToolCall>,
    finalizing: HashSet<TurnId>,
    records: HashMap<String, Value>,
    admissions: HashMap<MessageId, (Value, AdmissionReceipt)>,
    pending_inputs: std::collections::VecDeque<i64>,
    commands: HashMap<(String, uuid::Uuid), Value>,
    target_cancels: HashMap<MessageId, i64>,
    terminal: HashMap<TurnId, Event>,
}

impl MemoryHosting {
    fn new_session(id: SessionId) -> Arc<MemorySession> {
        Arc::new(MemorySession {
            id,
            renew_poke: Arc::default(),
            control: tokio::sync::Mutex::new(Control::default()),
            log: InMemoryEventLog::new(),
            services: HostBackends::in_memory(),
        })
    }
    fn session(&self, id: SessionId) -> Arc<MemorySession> {
        self.sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id)
            .cloned()
            .unwrap_or_else(|| Self::new_session(id))
    }
    fn ports(&self, id: SessionId) -> Ports {
        Ports {
            session: self.session(id),
            fence: Arc::new(RwLock::new(None)),
        }
    }
}

impl Control {
    fn binding(&self, id: SessionId) -> Result<SessionBinding> {
        self.binding
            .clone()
            .ok_or(AgentLoopError::SessionNotFound(id))
    }
    fn owns(&self, holder: &str, fence: u64) -> bool {
        self.holder.as_deref() == Some(holder)
            && self.fence == fence
            && self.expires.is_some_and(|expires| expires > Instant::now())
    }
}

#[async_trait]
impl SessionHosting for MemoryHosting {
    async fn resolve_waits(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        target: TurnId,
        approvals: Vec<ApprovalCommit>,
        results: Vec<ToolCompletedData>,
    ) -> Result<()> {
        let session = self.session(id);
        let mut state = session.control.lock().await;
        state.binding(id)?;
        let fingerprint = json!({"turn_id":target,"approvals":approvals,"results":results});
        let key = ("wait_resolution".into(), command);
        if let Some(previous) = state.commands.get(&key) {
            return if previous == &fingerprint {
                Ok(())
            } else {
                Err(AgentLoopError::config("wait resolution identity conflict"))
            };
        }
        if state.open != Some(target) {
            return Err(AgentLoopError::config(
                "wait resolution targets another turn",
            ));
        }
        let batch = if approvals.is_empty() {
            Vec::new()
        } else {
            prepare_approvals(
                &approvals,
                &state.pending,
                target,
                everruns_contracts::rt::utc_now(),
            )?
        };
        if !results.is_empty() {
            validate_tool_resolutions(&results, &state.pending, everruns_contracts::rt::utc_now())?;
        }
        if batch.is_empty() && results.is_empty() {
            return Err(AgentLoopError::config("empty wait resolution"));
        }
        let events = all_events(&session, None).await?;
        let input = events
            .iter()
            .rev()
            .find(|event| event.context.turn_id == Some(target))
            .and_then(|event| event.context.input_message_id)
            .ok_or_else(|| AgentLoopError::store("open turn has no input"))?;
        let mut last = 0;
        for prepared in batch {
            session
                .services
                .storage_store
                .set_value(id, &prepared.key, &prepared.record)
                .await?;
            session
                .log
                .append(EventRequest::new(
                    id,
                    EventContext::turn(target, input),
                    prepared.result,
                ))
                .await
                .map_err(core_error)?;
            let event = session
                .log
                .append(EventRequest::new(
                    id,
                    EventContext::turn(target, input),
                    InputMessageData::new(prepared.spoken),
                ))
                .await
                .map_err(core_error)?;
            state
                .pending
                .retain(|call| call.id != prepared.expected_request.id);
            last = event.sequence.unwrap_or(0);
        }
        for result in results {
            state.pending.retain(|call| call.id != result.tool_call_id);
            let event = session
                .log
                .append(EventRequest::new(
                    id,
                    EventContext::turn(target, input),
                    result,
                ))
                .await
                .map_err(core_error)?;
            last = event.sequence.unwrap_or(0);
        }
        if state.pending.is_empty() {
            state.inbox = i64::from(last);
            state.resume_generation += 1;
            state.runnable = true;
        }
        state.commands.insert(key, fingerprint);
        Ok(())
    }
    async fn submit_approvals(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        target: TurnId,
        decisions: Vec<everruns_contracts::execution_api::ToolApprovalAnswer>,
    ) -> Result<everruns_contracts::execution_api::SubmitToolApprovalsResponse> {
        let session = self.session(id);
        let mut state = session.control.lock().await;
        state.binding(id)?;
        let fingerprint = json!({"turn_id":target,"decisions":decisions});
        let key = ("approval_submission".into(), command);
        let receipt_key = format!("approval_submission/{command}");
        if let Some(previous) = state.commands.get(&key) {
            if previous != &fingerprint {
                return Err(AgentLoopError::config("approval command identity conflict"));
            }
            return serde_json::from_value(
                state
                    .records
                    .get(&receipt_key)
                    .cloned()
                    .ok_or_else(|| AgentLoopError::store("approval receipt missing"))?,
            )
            .map_err(core_error);
        }
        if state.open != Some(target) {
            return Err(AgentLoopError::config(
                "approval submission targets another turn",
            ));
        }
        let (approvals, response) =
            prepare_approval_submission(&decisions, &state.pending, target)?;
        let batch = prepare_approvals(
            &approvals,
            &state.pending,
            target,
            everruns_contracts::rt::utc_now(),
        )?;
        let events = all_events(&session, None).await?;
        let input = events
            .iter()
            .rev()
            .find(|event| event.context.turn_id == Some(target))
            .and_then(|event| event.context.input_message_id)
            .ok_or_else(|| AgentLoopError::store("open turn has no input"))?;
        let mut last = 0;
        for prepared in batch {
            session
                .services
                .storage_store
                .set_value(id, &prepared.key, &prepared.record)
                .await?;
            session
                .log
                .append(EventRequest::new(
                    id,
                    EventContext::turn(target, input),
                    prepared.result,
                ))
                .await
                .map_err(core_error)?;
            let spoken = session
                .log
                .append(EventRequest::new(
                    id,
                    EventContext::turn(target, input),
                    InputMessageData::new(prepared.spoken),
                ))
                .await
                .map_err(core_error)?;
            state
                .pending
                .retain(|call| call.id != prepared.expected_request.id);
            last = spoken.sequence.unwrap_or(0);
        }
        if state.pending.is_empty() {
            state.inbox = i64::from(last);
            state.resume_generation += 1;
            state.runnable = true;
        }
        state.commands.insert(key, fingerprint);
        state.records.insert(receipt_key, json!(response));
        Ok(response)
    }
    async fn configure(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        patch: SessionConfiguration,
    ) -> Result<()> {
        patch.validate()?;
        let session = self.session(id);
        let mut state = session.control.lock().await;
        state.binding(id)?;
        let fingerprint = json!(patch);
        let key = ("configuration".into(), command);
        if let Some(previous) = state.commands.get(&key) {
            return if previous == &fingerprint {
                Ok(())
            } else {
                Err(AgentLoopError::config(
                    "configuration command identity conflict",
                ))
            };
        }
        let mut configuration: SessionConfiguration = serde_json::from_value(
            state
                .records
                .get("configuration")
                .cloned()
                .unwrap_or(json!({})),
        )
        .map_err(core_error)?;
        configuration.merge(patch);
        state
            .records
            .insert("configuration".into(), json!(configuration));
        state.commands.insert(key, fingerprint);
        Ok(())
    }
    async fn seed_history(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        messages: Vec<RuntimeMessage>,
    ) -> Result<bool> {
        validate_history_seed(&messages)?;
        let session = self.session(id);
        let mut state = session.control.lock().await;
        state.binding(id)?;
        let fingerprint = json!(messages);
        let key = ("seed".into(), command);
        if let Some(previous) = state.commands.get(&key) {
            if previous != &fingerprint {
                return Err(AgentLoopError::config(
                    "history seed command identity conflict",
                ));
            }
            return Ok(state
                .records
                .get(&format!("seed/{command}"))
                .and_then(Value::as_bool)
                .unwrap_or(false));
        }
        let events = all_events(&session, None).await?;
        let empty = !events.iter().any(|event| {
            matches!(
                event.data,
                EventData::InputMessage(_) | EventData::OutputMessageCompleted(_)
            )
        });
        if empty {
            for message in messages {
                let request = if message.role == everruns_core::message::RuntimeMessageRole::Agent {
                    EventRequest::new(
                        id,
                        EventContext::empty(),
                        everruns_core::events::OutputMessageCompletedData::new(message),
                    )
                } else {
                    EventRequest::new(id, EventContext::empty(), InputMessageData::new(message))
                };
                session.log.append(request).await.map_err(core_error)?;
            }
        }
        state.commands.insert(key, fingerprint);
        state
            .records
            .insert(format!("seed/{command}"), json!(empty));
        Ok(empty)
    }
    async fn query_snapshot(&self, id: SessionId) -> Result<SessionQuerySnapshot> {
        use everruns_core::host::{EventReadLimit, EventReadRequest};
        let session = self.session(id);
        let state = session.control.lock().await;
        let binding = state.binding(id)?;
        let page = session
            .log
            .read_page(EventReadRequest::new(
                id,
                EventReadLimit::new(1).map_err(core_error)?,
            ))
            .await
            .map_err(core_error)?;
        let execution = session.services.session_store.get_session(id).await?;
        Ok(SessionQuerySnapshot {
            configuration: serde_json::from_value(
                state
                    .records
                    .get("configuration")
                    .cloned()
                    .unwrap_or(json!({})),
            )
            .map_err(core_error)?,
            binding,
            high_watermark: page.snapshot_high_watermark(),
            execution,
            rejected_inputs: state
                .records
                .keys()
                .filter_map(|key| key.strip_prefix("rejected/"))
                .map(|key| {
                    key.parse()
                        .map_err(|_| AgentLoopError::store("invalid rejection receipt identity"))
                })
                .collect::<Result<_>>()?,
        })
    }
    async fn approve_many(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        approvals: Vec<ApprovalCommit>,
    ) -> Result<()> {
        let session = self.session(id);
        let mut state = session.control.lock().await;
        state.binding(id)?;
        let fingerprint = json!({"approvals":approvals});
        if let Some(previous) = state.commands.get(&("approval".into(), command)) {
            return if previous == &fingerprint {
                Ok(())
            } else {
                Err(AgentLoopError::config("approval command identity conflict"))
            };
        }
        let turn = state
            .open
            .ok_or_else(|| AgentLoopError::config("session has no open turn"))?;
        let batch = prepare_approvals(
            &approvals,
            &state.pending,
            turn,
            everruns_contracts::rt::utc_now(),
        )?;
        let events = all_events(&session, None).await?;
        let input = events
            .iter()
            .rev()
            .find(|event| event.context.turn_id == Some(turn))
            .and_then(|event| event.context.input_message_id)
            .ok_or_else(|| AgentLoopError::store("open turn has no input"))?;
        let mut last_sequence = 0;
        for prepared in batch {
            session
                .services
                .storage_store
                .set_value(id, &prepared.key, &prepared.record)
                .await?;
            session
                .log
                .append(EventRequest::new(
                    id,
                    EventContext::turn(turn, input),
                    prepared.result,
                ))
                .await
                .map_err(core_error)?;
            let spoken = session
                .log
                .append(EventRequest::new(
                    id,
                    EventContext::turn(turn, input),
                    InputMessageData::new(prepared.spoken),
                ))
                .await
                .map_err(core_error)?;
            state
                .pending
                .retain(|call| call.id != prepared.expected_request.id);
            last_sequence = spoken.sequence.unwrap_or(0);
        }
        state
            .commands
            .insert(("approval".into(), command), fingerprint);
        if state.pending.is_empty() {
            state.inbox = i64::from(last_sequence);
            state.resume_generation += 1;
            state.runnable = true;
        }
        Ok(())
    }
    async fn create(&self, id: SessionId, binding: &SessionBinding) -> Result<()> {
        let session = {
            let mut sessions = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
            if sessions.len() >= 100_000 && !sessions.contains_key(&id) {
                return Err(AgentLoopError::store("memory session capacity reached"));
            }
            sessions
                .entry(id)
                .or_insert_with(|| Self::new_session(id))
                .clone()
        };
        let mut state = session.control.lock().await;
        if state.binding.as_ref().is_some_and(|bound| bound != binding) {
            return Err(AgentLoopError::config("session binding conflict"));
        }
        if state.binding.is_none() {
            state.created_at = everruns_contracts::rt::utc_now_iso();
        }
        state.binding = Some(binding.clone());
        Ok(())
    }
    async fn status(&self, id: SessionId) -> Result<HostingStatus> {
        let session = self.session(id);
        let state = session.control.lock().await;
        Ok(HostingStatus {
            now_ms: everruns_contracts::rt::utc_now().timestamp_millis(),
            wake_at: state.wake_at,
            created_at: state.created_at.clone(),
            binding: state.binding(id)?,
            runnable: state.runnable,
            suspended: state.suspended,
            suspension_reason: if state.suspended {
                state
                    .records
                    .get("suspension")
                    .and_then(|value| value["reason"].as_str().map(str::to_owned))
            } else {
                None
            },
            open_turn: state.open,
            waiting: state.pending.clone(),
        })
    }
    async fn admit(
        &self,
        id: SessionId,
        message: RuntimeMessage,
        cancelled: bool,
    ) -> Result<AdmissionReceipt> {
        let session = self.session(id);
        let mut state = session.control.lock().await;
        state.binding(id)?;
        if message.role != everruns_core::RuntimeMessageRole::User {
            return Err(AgentLoopError::config("admission requires user input"));
        }
        let mut fingerprint =
            serde_json::to_value(&message).map_err(|_| AgentLoopError::config("invalid input"))?;
        fingerprint.as_object_mut().map(|object| {
            object.remove("created_at");
            object.remove("updated_at");
        });
        if cancelled {
            fingerprint = json!({"message":fingerprint,"cancelled":true});
        }
        if let Some((previous, receipt)) = state.admissions.get(&message.id) {
            return if previous == &fingerprint {
                Ok(receipt.clone())
            } else {
                Err(AgentLoopError::config(
                    "command identity reused with different input",
                ))
            };
        }
        if state.admissions.len() >= 100_000 {
            return Err(AgentLoopError::store("session admission capacity reached"));
        }
        while state
            .pending_inputs
            .front()
            .is_some_and(|sequence| *sequence <= state.consumed)
        {
            state.pending_inputs.pop_front();
        }
        if state.pending_inputs.len() >= 256 {
            return Err(AgentLoopError::store("session inbox is full"));
        }
        let message_id = message.id;
        let event = session
            .log
            .append(EventRequest::new(
                id,
                EventContext::empty(),
                InputMessageData::new(message),
            ))
            .await
            .map_err(core_error)?;
        let sequence = event
            .sequence
            .ok_or_else(|| AgentLoopError::store("admitted event has no sequence"))?;
        let receipt = AdmissionReceipt {
            message_id,
            sequence,
        };
        state
            .admissions
            .insert(message_id, (fingerprint, receipt.clone()));
        state.pending_inputs.push_back(i64::from(sequence));
        if cancelled {
            state.target_cancels.insert(message_id, i64::from(sequence));
        }
        state.inbox = i64::from(sequence);
        if state.next_input <= state.consumed {
            state.next_input = state.inbox;
        }
        state.runnable = true;
        Ok(receipt)
    }
    async fn receipt(&self, id: SessionId, message: MessageId) -> Result<Option<AdmissionReceipt>> {
        let session = self.session(id);
        let state = session.control.lock().await;
        state.binding(id)?;
        Ok(state
            .admissions
            .get(&message)
            .map(|(_, receipt)| receipt.clone()))
    }
    fn reader(&self, id: SessionId) -> HostBackends {
        self.ports(id).backends()
    }
    async fn record(&self, id: SessionId, key: &str) -> Result<Option<Value>> {
        SessionEpoch::record(&self.ports(id), key).await
    }
    async fn resolve(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        target: everruns_contracts::typed_id::TurnId,
        results: Vec<ToolCompletedData>,
    ) -> Result<()> {
        let session = self.session(id);
        let mut state = session.control.lock().await;
        state.binding(id)?;
        let fingerprint = json!({"turn_id":target,"results":results});
        if let Some(previous) = state.commands.get(&("resolution".into(), command)) {
            return if previous == &fingerprint {
                Ok(())
            } else {
                Err(AgentLoopError::config("resolution identity conflict"))
            };
        }
        validate_tool_resolutions(&results, &state.pending, everruns_contracts::rt::utc_now())?;
        // Resolve only calls still requested in the canonical open turn.
        let turn = state
            .open
            .ok_or_else(|| AgentLoopError::config("session has no open turn"))?;
        let events = all_events(&session, None).await?;
        let mut pending = HashMap::new();
        for event in &events {
            if event.context.turn_id != Some(turn) {
                continue;
            }
            match &event.data {
                EventData::ToolCallRequested(data) => {
                    pending = data
                        .tool_calls
                        .iter()
                        .map(|call| (call.id.clone(), call.name.clone()))
                        .collect();
                }
                EventData::ToolCompleted(data) => {
                    pending.remove(&data.tool_call_id);
                }
                _ => {}
            }
        }
        if state.open != Some(target) {
            return Err(AgentLoopError::config(
                "tool resolution targets another turn",
            ));
        }
        let mut seen = HashSet::new();
        if results.is_empty()
            || results.iter().any(|result| {
                pending.get(&result.tool_call_id) != Some(&result.tool_name)
                    || !seen.insert(result.tool_call_id.clone())
            })
        {
            return Err(AgentLoopError::config(
                "tool result does not match a pending call",
            ));
        }
        let input = events
            .iter()
            .rev()
            .find(|event| event.context.turn_id == Some(turn))
            .and_then(|event| event.context.input_message_id)
            .ok_or_else(|| AgentLoopError::store("open turn has no input identity"))?;
        let mut last = 0;
        for result in results {
            pending.remove(&result.tool_call_id);
            state.pending.retain(|call| call.id != result.tool_call_id);
            let event = session
                .log
                .append(EventRequest::new(
                    id,
                    EventContext::turn(turn, input),
                    result,
                ))
                .await
                .map_err(core_error)?;
            last = event.sequence.unwrap_or(0);
        }
        state
            .commands
            .insert(("resolution".into(), command), fingerprint);
        if pending.is_empty() {
            state.resume_generation += 1;
            state.inbox = i64::from(last);
            state.runnable = true;
        }
        Ok(())
    }
    async fn cancel(&self, id: SessionId, command: uuid::Uuid, input: MessageId) -> Result<()> {
        let session = self.session(id);
        let mut state = session.control.lock().await;
        state.binding(id)?;
        let fingerprint = json!({"cancel":input});
        if let Some(previous) = state.commands.get(&("cancel".into(), command)) {
            return if previous == &fingerprint {
                Ok(())
            } else {
                Err(AgentLoopError::config("cancellation identity conflict"))
            };
        }
        if !state.admissions.contains_key(&input) {
            return Err(AgentLoopError::config("admission not found"));
        }
        let position = state.inbox;
        state.target_cancels.insert(input, position);
        if let Some(assignment) = state.records.get(&format!("assignment/{input}")) {
            let assignment: super::TurnAssignment = serde_json::from_value(assignment.clone())
                .map_err(|_| AgentLoopError::store("invalid turn assignment"))?;
            if (state.open.is_none() || state.open == Some(assignment.turn_id))
                && !state.terminal.contains_key(&assignment.turn_id)
                && !state.records.contains_key(&format!("rejected/{input}"))
            {
                state.cancel = Some(position);
                state.cancel_reported = false;
                state.runnable = true;
                session.renew_poke.notify_one();
            }
        }
        state
            .commands
            .insert(("cancel".into(), command), fingerprint);
        Ok(())
    }
    async fn claim(
        &self,
        id: SessionId,
        holder: &str,
        ttl: Duration,
    ) -> Result<Option<SessionActivation>> {
        let session = self.session(id);
        let mut state = session.control.lock().await;
        let binding = state.binding(id)?;
        if state
            .wake_at
            .is_some_and(|at| at <= everruns_contracts::rt::utc_now().timestamp_millis())
        {
            state.wake_at = None;
            state.resume_generation += 1;
            state.runnable = true;
        }
        if !state.runnable
            || state.suspended
            || state
                .expires
                .is_some_and(|expires| expires > Instant::now())
        {
            return Ok(None);
        }
        state.holder = Some(holder.to_owned());
        state.fence += 1;
        state.expires = Some(Instant::now() + ttl);
        state.attempts += 1;
        state.cancel_reported = state.cancel.is_some();
        state.resume_claimed = state.resume_generation;
        let actor = LeasedActor {
            session_id: id,
            org_id: 1,
            fence: state.fence,
            inbox_seq: state.inbox,
            cancel_inbox_seq: state.cancel,
            consumed_seq: state.consumed,
            next_input_seq: state.next_input,
            attempts: state.attempts,
        };
        Ok(Some(SessionActivation {
            binding,
            actor,
            epoch: Arc::new(Ports {
                session: session.clone(),
                fence: Arc::new(RwLock::new(Some(ActorFence::new(id, holder, state.fence)))),
            }),
        }))
    }
    fn process_scheduler(&self) -> bool {
        true
    }
}

#[derive(Clone)]
struct Ports {
    session: Arc<MemorySession>,
    fence: Arc<RwLock<Option<ActorFence>>>,
}

fn core_error(error: impl std::fmt::Display) -> AgentLoopError {
    AgentLoopError::store(error.to_string())
}
fn log_error(error: impl std::fmt::Display) -> EventLogError {
    EventLogError::Backend {
        detail: error.to_string(),
    }
}

impl Ports {
    fn check(&self, state: &Control) -> Result<()> {
        // THREAT[TM-DURABLE-012]: callers hold the authority gate while checking
        // ownership and committing; lease expiry fences delayed writers too.
        let fence = self.fence.read().unwrap_or_else(|p| p.into_inner());
        if fence
            .as_ref()
            .is_some_and(|fence| !fence.is_lost() && state.owns(&fence.holder, fence.fence))
        {
            return Ok(());
        }
        let error = actor_lease_lost(
            self.session.id,
            "memory authority ownership expired or absent",
        );
        if let Some(fence) = &*fence {
            fence.observe::<()>(&Err(actor_lease_lost(self.session.id, "ownership lost")));
        }
        Err(error)
    }
    fn scope(&self, id: SessionId) -> Result<()> {
        if id != self.session.id {
            return Err(AgentLoopError::config("cross-session authority access"));
        }
        Ok(())
    }
    async fn append(
        &self,
        request: EventRequest,
        expected: Option<Option<i32>>,
    ) -> std::result::Result<Event, EventLogError> {
        self.scope(request.session_id).map_err(log_error)?;
        let mut state = self.session.control.lock().await;
        self.check(&state).map_err(log_error)?;
        let turn = request.context.turn_id;
        let terminal = matches!(
            request.data,
            EventData::TurnCompleted(_)
                | EventData::TurnFailed(_)
                | EventData::TurnCancelled(_)
                | EventData::TurnSealed(_)
        );
        if terminal && let Some(previous) = turn.and_then(|turn| state.terminal.get(&turn)) {
            if serde_json::to_value(&previous.data).ok() == serde_json::to_value(&request.data).ok()
            {
                return Ok(previous.clone());
            }
            return Err(log_error("conflicting turn terminal outcome"));
        }
        let event = match expected {
            None => self.session.log.append(request).await?,
            Some(expected) => {
                self.session
                    .log
                    .append_conditional(request, expected)
                    .await?
            }
        };
        if matches!(event.data, EventData::TurnStarted(_)) {
            state.open = turn;
        }
        match &event.data {
            EventData::ToolCallRequested(data) => state.pending = data.tool_calls.clone(),
            EventData::ToolCompleted(data) => {
                state.pending.retain(|call| call.id != data.tool_call_id)
            }
            _ => {}
        }
        if terminal && let Some(turn) = turn {
            state.open = None;
            state.pending.clear();
            state.finalizing.insert(turn);
            state.terminal.insert(turn, event.clone());
        }
        Ok(event)
    }
}

#[async_trait]
impl SessionEpoch for Ports {
    fn renew_poke(&self) -> Option<Arc<tokio::sync::Notify>> {
        Some(self.session.renew_poke.clone())
    }
    async fn schedule(&self, wake_at: Option<i64>) -> Result<()> {
        let mut state = self.session.control.lock().await;
        self.check(&state)?;
        state.wake_at = wake_at;
        Ok(())
    }
    fn backends(&self) -> HostBackends {
        let mut backends = HostBackends::in_memory();
        backends.event_log = Arc::new(self.clone());
        backends.session_store = Arc::new(self.clone());
        backends.storage_store = Arc::new(self.clone());
        backends.compaction_checkpoint_store = Arc::new(self.clone());
        backends
    }
    fn actor_store(&self) -> Arc<dyn SessionActorStore> {
        Arc::new(self.clone())
    }
    fn observe_fence(&self, fence: &ActorFence) {
        let mut bound = self.fence.write().unwrap_or_else(|p| p.into_inner());
        if bound
            .as_ref()
            .is_some_and(|current| current.holder == fence.holder && current.fence == fence.fence)
        {
            *bound = Some(fence.clone());
        }
    }
    async fn record(&self, key: &str) -> Result<Option<Value>> {
        Ok(self.session.control.lock().await.records.get(key).cloned())
    }
    async fn records(&self, prefix: &str) -> Result<Vec<(String, Value)>> {
        let state = self.session.control.lock().await;
        let records: Vec<_> = state
            .records
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .take(4097)
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        if records.len() > 4096 {
            return Err(AgentLoopError::store(
                "lifecycle receipt scan limit exceeded",
            ));
        }
        Ok(records)
    }
    async fn commit(&self, key: &str, value: Value, absent: bool) -> Result<bool> {
        let mut state = self.session.control.lock().await;
        self.check(&state)?;
        if absent && state.records.contains_key(key) {
            return Ok(false);
        }
        if let Some(input) = key
            .strip_prefix("assignment/")
            .and_then(|key| key.parse::<MessageId>().ok())
            && let Some(position) = state.target_cancels.get(&input).copied()
        {
            let assignment: super::TurnAssignment = serde_json::from_value(value.clone())
                .map_err(|_| AgentLoopError::store("invalid turn assignment"))?;
            if state.open.is_none() || state.open == Some(assignment.turn_id) {
                state.cancel = Some(position);
                state.cancel_reported = false;
                state.runnable = true;
            }
        }
        if let Some(turn) = key
            .strip_prefix("outcome/")
            .and_then(|key| key.parse::<TurnId>().ok())
        {
            state.finalizing.remove(&turn);
        }
        state.records.insert(key.to_owned(), value);
        Ok(true)
    }
    async fn suspend(&self, reason: &str) -> Result<()> {
        let mut state = self.session.control.lock().await;
        self.check(&state)?;
        state.suspended = true;
        state
            .records
            .insert("suspension".into(), json!({"reason":reason}));
        Ok(())
    }
}

#[async_trait]
impl EventReader for Ports {
    async fn read_page(
        &self,
        request: EventReadRequest,
    ) -> std::result::Result<EventPage, EventLogError> {
        self.scope(request.session_id()).map_err(log_error)?;
        self.session.log.read_page(request).await
    }
}
#[async_trait]
impl EventLog for Ports {
    async fn append(&self, request: EventRequest) -> std::result::Result<Event, EventLogError> {
        Ports::append(self, request, None).await
    }
    async fn append_conditional(
        &self,
        request: EventRequest,
        expected: Option<i32>,
    ) -> std::result::Result<Event, EventLogError> {
        Ports::append(self, request, Some(expected)).await
    }
    fn durability(&self) -> EventDurability {
        EventDurability::Volatile
    }
}

async fn all_events(session: &MemorySession, after: Option<i32>) -> Result<Vec<Event>> {
    let mut events = Vec::new();
    let mut request = EventReadRequest::from_cursor(
        EventCursor::after(session.id, after.unwrap_or(0)).map_err(core_error)?,
        EventReadLimit::default(),
    );
    loop {
        let page = session.log.read_page(request).await.map_err(core_error)?;
        events.extend(page.events);
        if events.len() > 100_000 {
            return Err(AgentLoopError::store(
                "session journal recovery limit exceeded",
            ));
        }
        match page.next_cursor {
            Some(cursor) => {
                request = EventReadRequest::from_cursor(cursor, EventReadLimit::default())
            }
            None => return Ok(events),
        }
    }
}

#[async_trait]
impl SessionActorStore for Ports {
    async fn renew_actor_lease(
        &self,
        id: SessionId,
        holder: &str,
        fence: u64,
        ttl: Duration,
        consumed: i64,
    ) -> Result<Option<ActorRenewal>> {
        self.scope(id)?;
        let mut state = self.session.control.lock().await;
        if !state.owns(holder, fence) {
            return Ok(None);
        }
        state.consumed = state.consumed.max(consumed);
        state.expires = Some(Instant::now() + ttl);
        state.cancel_reported = state.cancel.is_some();
        Ok(Some(ActorRenewal {
            inbox_seq: state.inbox,
            cancel_inbox_seq: state.cancel,
        }))
    }
    async fn release_actor(
        &self,
        id: SessionId,
        holder: &str,
        fence: u64,
        consumed: i64,
    ) -> Result<ActorRelease> {
        self.scope(id)?;
        let mut state = self.session.control.lock().await;
        if !state.owns(holder, fence) {
            return Ok(ActorRelease::LeaseLost);
        }
        state.consumed = state.consumed.max(consumed);
        state.holder = None;
        state.expires = None;
        state.attempts = 0;
        state.runnable = !state.finalizing.is_empty()
            || state.resume_generation > state.resume_claimed
            || state.inbox > state.consumed
            || (state.cancel.is_some() && !state.cancel_reported);
        if state.cancel_reported {
            state.cancel = None;
        }
        state.cancel_reported = false;
        Ok(ActorRelease::Released {
            still_runnable: state.runnable,
        })
    }
    async fn read_actor_journal(
        &self,
        id: SessionId,
        after: Option<i32>,
        limit: u32,
    ) -> Result<ActorJournal> {
        self.scope(id)?;
        let mut events = all_events(&self.session, after).await?;
        let head = events.last().and_then(|event| event.sequence);
        if after.is_none()
            && let Some(start) = events
                .iter()
                .rposition(|event| matches!(event.data, EventData::TurnStarted(_)))
        {
            events.drain(..start);
        }
        let limit = if limit == 0 {
            256
        } else {
            limit.min(1024) as usize
        };
        let has_more = events.len() > limit;
        events.truncate(limit);
        Ok(ActorJournal {
            events,
            has_more,
            head_sequence: head,
        })
    }
}

#[async_trait]
impl SessionStore for Ports {
    async fn get_session(&self, id: SessionId) -> Result<Option<ExecutionSession>> {
        self.scope(id)?;
        let state = self.session.control.lock().await;
        let mut record = self.session.services.session_store.get_session(id).await?;
        let configuration: SessionConfiguration = serde_json::from_value(
            state
                .records
                .get("configuration")
                .cloned()
                .unwrap_or(json!({})),
        )
        .map_err(core_error)?;
        if let Some(record) = record.as_mut() {
            configuration.apply(record);
        }
        Ok(record)
    }
}
#[async_trait]
impl RuntimeSessionStore for Ports {
    async fn add_session(&self, session: ExecutionSession) -> Result<()> {
        self.scope(session.id)?;
        let state = self.session.control.lock().await;
        self.check(&state)?;
        self.session
            .services
            .session_store
            .add_session(session)
            .await
    }
}
#[async_trait]
impl SessionMutator for Ports {
    async fn update_session_title(&self, id: SessionId, title: String) -> Result<ExecutionSession> {
        self.scope(id)?;
        let state = self.session.control.lock().await;
        self.check(&state)?;
        self.session
            .services
            .session_store
            .update_session_title(id, title)
            .await
    }
}
#[async_trait]
impl SessionStorageStore for Ports {
    async fn get_value(&self, id: SessionId, key: &str) -> Result<Option<String>> {
        self.scope(id)?;
        self.session.services.storage_store.get_value(id, key).await
    }
    async fn set_value(&self, id: SessionId, key: &str, value: &str) -> Result<()> {
        self.scope(id)?;
        let state = self.session.control.lock().await;
        self.check(&state)?;
        self.session
            .services
            .storage_store
            .set_value(id, key, value)
            .await
    }
    async fn take_value(&self, id: SessionId, key: &str) -> Result<Option<String>> {
        self.scope(id)?;
        let state = self.session.control.lock().await;
        self.check(&state)?;
        self.session
            .services
            .storage_store
            .take_value(id, key)
            .await
    }
    async fn delete_value(&self, id: SessionId, key: &str) -> Result<bool> {
        self.scope(id)?;
        let state = self.session.control.lock().await;
        self.check(&state)?;
        self.session
            .services
            .storage_store
            .delete_value(id, key)
            .await
    }
    async fn list_keys(&self, id: SessionId) -> Result<Vec<KeyInfo>> {
        self.scope(id)?;
        self.session.services.storage_store.list_keys(id).await
    }
    async fn set_secret(&self, id: SessionId, key: &str, value: &str) -> Result<()> {
        self.scope(id)?;
        let state = self.session.control.lock().await;
        self.check(&state)?;
        self.session
            .services
            .storage_store
            .set_secret(id, key, value)
            .await
    }
    async fn get_secret(&self, id: SessionId, key: &str) -> Result<Option<String>> {
        self.scope(id)?;
        self.session
            .services
            .storage_store
            .get_secret(id, key)
            .await
    }
    async fn delete_secret(&self, id: SessionId, key: &str) -> Result<bool> {
        self.scope(id)?;
        let state = self.session.control.lock().await;
        self.check(&state)?;
        self.session
            .services
            .storage_store
            .delete_secret(id, key)
            .await
    }
    async fn list_secrets(&self, id: SessionId) -> Result<Vec<SecretInfo>> {
        self.scope(id)?;
        self.session.services.storage_store.list_secrets(id).await
    }
}
#[async_trait]
impl CompactionCheckpointStore for Ports {
    async fn get_latest(
        &self,
        id: SessionId,
        provider: &str,
        model: &str,
    ) -> Result<Option<CompactionCheckpoint>> {
        self.scope(id)?;
        self.session
            .services
            .compaction_checkpoint_store
            .get_latest(id, provider, model)
            .await
    }
    async fn get_latest_format(
        &self,
        id: SessionId,
        provider: &str,
        model: &str,
        format: u32,
    ) -> Result<Option<CompactionCheckpoint>> {
        self.scope(id)?;
        self.session
            .services
            .compaction_checkpoint_store
            .get_latest_format(id, provider, model, format)
            .await
    }
    async fn install(&self, checkpoint: CompactionCheckpoint) -> Result<bool> {
        self.scope(checkpoint.session_id)?;
        let state = self.session.control.lock().await;
        self.check(&state)?;
        self.session
            .services
            .compaction_checkpoint_store
            .install(checkpoint)
            .await
    }
    async fn get_proactive_attempt(
        &self,
        id: SessionId,
        provider: &str,
        model: &str,
    ) -> Result<Option<ProactiveCompactionAttempt>> {
        self.scope(id)?;
        self.session
            .services
            .compaction_checkpoint_store
            .get_proactive_attempt(id, provider, model)
            .await
    }
    async fn record_proactive_attempt(
        &self,
        id: SessionId,
        provider: &str,
        model: &str,
        attempt: ProactiveCompactionAttempt,
    ) -> Result<()> {
        self.scope(id)?;
        let state = self.session.control.lock().await;
        self.check(&state)?;
        self.session
            .services
            .compaction_checkpoint_store
            .record_proactive_attempt(id, provider, model, attempt)
            .await
    }
}
