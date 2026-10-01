//! Durable orchestration of one Everruns turn through an Agents API session.
//!
//! The driver persists a write-ahead [`AgentsApiCheckpoint`] before every
//! provider call and local effect:
//!
//! - **Create.** The create attempt is saved before `POST /agents/sessions`.
//!   The provider does not deduplicate creates, so an uncertain create is
//!   adopted by its metadata instead of repeated.
//! - **Input.** Follow-up input is staged with an idempotency key before it is
//!   sent; a retry carries the same key and the provider drops the duplicate.
//!   The provider root turns that existed before the input identify the turn
//!   it created.
//! - **Items.** Each provider item maps to a local record through a saved
//!   correlation. A record moves `open -> completing -> completed`; after a
//!   crash in `completing` the event log is checked before emitting again, so
//!   duplicate or replayed provider events cannot duplicate local messages.
//! - **Tool results.** A function call is claimed before execution, its result
//!   saved before submission, and submission carries an idempotency key. A
//!   claimed call found after a crash reuses the recorded result instead of
//!   running the tool again.
//! - **Stream.** Provider streams do not replay. After a disconnect the driver
//!   opens a new stream, then reconciles from the saved session, items, and
//!   turn. Before reporting the outcome it reconciles once more, so a missing
//!   event cannot lose a message.
//! - **Terminal.** The outcome is saved before the activity returns, so a
//!   replayed activity returns it without calling the provider again.
//!
//! Everruns policy applies at the tool and output boundaries (EVE-1124):
//!
//! - **Pauses.** A call the tool pipeline parks (an approval, a client-side
//!   tool, a connection setup) is saved as parked and the activity returns
//!   [`AgentsApiTurnOutcome::Paused`]. The provider's required action stays
//!   open; nothing is submitted. The resumed turn (a later reason iteration)
//!   resolves it: an approval runs the call again under a fresh local id, a
//!   denial or expiry submits a failed result, a client answer is submitted.
//! - **Stops.** An output guardrail that trips on a remote message, a budget,
//!   or a blocked dependency saves a [`PolicyStop`] before its effects,
//!   cancels the provider turn, and ends the Everruns turn.
//!
//! Observability and cost (EVE-1125) live in `observe`: subagent turns,
//! provider-run tools, reasoning summaries, and managed compaction become
//! canonical events, and every provider turn that ends (completed, failed,
//! cancelled, or stopped by policy) is accounted once as an `llm.generation`
//! whose cost components name any amount nobody could price.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use everruns_core::agents_api_store::{
    AgentsApiCheckpoint, AgentsApiLease, AgentsApiStore, AgentsApiTurnCheckpoint, InputOutbox,
    ItemCorrelation, ItemKind, ItemState, OutboxState, ParkReason, PolicyStop, ReplacedMessage,
    ToolResultState,
};
use everruns_core::events::correlation::{
    PROVIDER_ITEM_ID, PROVIDER_SESSION_ID, PROVIDER_TRACE_URL, PROVIDER_TURN_ID, RUNTIME_BACKEND,
};
use everruns_core::events::{
    EventContext, EventRequest, ModelMetadata, OutputMessageCompletedData, OutputMessageDeltaData,
    OutputMessageStartedData, TokenUsage, ToolCompletedData, ToolDefinitionSummary,
    ToolStartedData,
};
use everruns_core::output_guardrail::TrippedGuardrail;
use everruns_core::{ContentPart, RuntimeMessage, mcp_tool_name};
use everruns_provider::execution_phase::ExecutionPhase;
use everruns_provider::tool_types::ToolCall;
use everruns_provider::typed_id::{MessageId, SessionId, TurnId};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

mod observe;
mod policy;

use super::{
    AgentsApiClient, AgentsApiError, AgentsApiEventStream, AgentsApiSessionConfig,
    FunctionCallAction, build_message_input, build_tool_result_input, is_subagent_event,
    message_item_text, provider_output_text, usage_from,
};

/// Metadata keys written on the provider session for adoption after an
/// uncertain create.
pub const METADATA_SESSION_KEY: &str = "everruns_session_id";
pub const METADATA_ATTEMPT_KEY: &str = "everruns_create_attempt";

/// Local attempts of one provider call before a call that keeps asking for a
/// user action is answered with a failure instead of another pause.
pub const MAX_CALL_ATTEMPTS: u32 = 3;
/// Policy-stop code of an output guardrail that replaced a remote message.
pub const OUTPUT_GUARDRAIL_STOP: &str = "output_guardrail";
/// Failure code when the provider turn ended while a call was parked.
pub const PARKED_CALL_EXPIRED: &str = "tool_action_expired";

/// One Everruns turn to drive through the provider.
#[derive(Clone, Debug)]
pub struct AgentsApiTurnRequest {
    pub org_id: i64,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub input_message_id: MessageId,
    /// Reason iteration of this activity. A parked call resolves only in a
    /// later iteration (the turn resumed), never in a replay of the one that
    /// parked it.
    pub iteration: u32,
    /// The turn's user input.
    pub input_text: String,
    /// Agent definition for a new provider session; `input` and `metadata`
    /// are filled by the driver.
    pub config: AgentsApiSessionConfig,
    /// Correlation context for every emitted event.
    pub event_context: EventContext,
    /// Provider type recorded on the turn's `llm.generation` (`openai`).
    pub provider: Option<String>,
    /// Everruns LLM provider whose credentials the turn uses. The provider
    /// session belongs to it: lifecycle work resolves its key to delete the
    /// session, and a turn on another provider starts a new session.
    pub provider_key: Option<String>,
    /// Tools the remote loop was offered, recorded on `llm.generation`.
    pub tools: Vec<ToolDefinitionSummary>,
}

/// How the root provider turn ended.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AgentsApiTurnOutcome {
    Completed {
        final_message_id: Option<MessageId>,
        final_text: String,
        /// Null provider usage stays unknown, never zero.
        usage: Option<TokenUsage>,
        tool_calls: u32,
    },
    Failed {
        code: Option<String>,
        message: String,
        /// Everruns authored the failure (a policy stop, an expired pause), so
        /// `message` is user-facing as is.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        policy: bool,
    },
    Cancelled,
    /// A call is parked on a user action; the turn resumes later. Never saved
    /// as an outcome: the resumed turn continues the same provider turn.
    Paused,
}

/// The Everruns event log as the driver sees it.
#[async_trait]
pub trait AgentsApiLedger: Send + Sync {
    /// Append one event to the session's log.
    async fn emit(&self, event: EventRequest) -> Result<(), AgentsApiError>;
    /// Whether the canonical message with this id is already recorded.
    async fn has_message(
        &self,
        session_id: SessionId,
        message_id: MessageId,
    ) -> Result<bool, AgentsApiError>;
    /// The recorded result of a tool call, if one is in the log.
    async fn tool_result(
        &self,
        session_id: SessionId,
        call_id: &str,
    ) -> Result<Option<Result<String, String>>, AgentsApiError>;
}

/// How one client function call left the tool pipeline.
#[derive(Clone, Debug, PartialEq)]
pub enum FunctionOutcome {
    /// A result to submit: the output, or the error text of a failed or
    /// denied call.
    Done(Result<String, String>),
    /// The call paused the turn; the provider's required action stays open.
    Parked(ParkReason),
}

/// The result of one batch through the tool pipeline.
#[derive(Clone, Debug, PartialEq)]
pub enum FunctionBatch {
    /// One outcome per call, in order.
    Outcomes(Vec<FunctionOutcome>),
    /// Policy forbids running any more tools in this turn (an exhausted
    /// budget, an archived agent). The turn stops with `message`.
    Halt { code: String, message: String },
}

/// Runs client functions through Everruns' tool pipeline and records their
/// results in the event log. The provider waits for the returned outputs.
///
/// Calls carry local ids: the provider call id for a first attempt, a fresh
/// one for a call run again after a pause.
#[async_trait]
pub trait AgentsApiFunctionExecutor: Send + Sync {
    async fn execute(&self, calls: &[ToolCall]) -> Result<FunctionBatch, AgentsApiError>;
}

/// Everruns output guardrails over the remote loop's assistant messages.
#[async_trait]
pub trait AgentsApiOutputPolicy: Send + Sync {
    /// Whether live text must be withheld until the completed message passes
    /// (an end-of-message guardrail is configured).
    fn withholds_deltas(&self) -> bool;
    /// Streaming guardrails over one message's accumulated live text.
    fn check_delta(
        &self,
        item_id: &str,
        accumulated: &str,
        delta: &str,
    ) -> Option<TrippedGuardrail>;
    /// Every guardrail over a completed message's full text.
    async fn check_message(&self, item_id: &str, text: &str) -> Option<TrippedGuardrail>;
}

/// Drives turns through the provider with durable, idempotent orchestration.
pub struct AgentsApiTurnDriver {
    client: AgentsApiClient,
    store: Arc<dyn AgentsApiStore>,
    ledger: Arc<dyn AgentsApiLedger>,
    executor: Arc<dyn AgentsApiFunctionExecutor>,
    output_policy: Option<Arc<dyn AgentsApiOutputPolicy>>,
    cancellation: Option<tokio::sync::watch::Receiver<bool>>,
    /// Consecutive stream reconnects without progress before giving up.
    max_idle_reconnects: u32,
    reconnect_backoff: Duration,
    heartbeat: Duration,
    /// Reads of a terminal turn whose usage is still null before it is
    /// reported as unknown (the provider fills usage late).
    usage_reads: u32,
    usage_backoff: Duration,
}

impl AgentsApiTurnDriver {
    pub fn new(
        client: AgentsApiClient,
        store: Arc<dyn AgentsApiStore>,
        ledger: Arc<dyn AgentsApiLedger>,
        executor: Arc<dyn AgentsApiFunctionExecutor>,
    ) -> Self {
        Self {
            client,
            store,
            ledger,
            executor,
            output_policy: None,
            cancellation: None,
            max_idle_reconnects: 5,
            reconnect_backoff: Duration::from_millis(250),
            heartbeat: Duration::from_secs(10),
            usage_reads: 4,
            usage_backoff: Duration::from_millis(500),
        }
    }

    /// Stop the turn when the durable task is cancelled or loses ownership.
    pub fn with_cancellation(mut self, receiver: tokio::sync::watch::Receiver<bool>) -> Self {
        self.cancellation = Some(receiver);
        self
    }

    /// Run output guardrails on the remote loop's assistant messages.
    pub fn with_output_policy(mut self, policy: Arc<dyn AgentsApiOutputPolicy>) -> Self {
        self.output_policy = Some(policy);
        self
    }

    /// Reconnect policy (tests shorten the backoff).
    pub fn with_reconnect_policy(mut self, max_idle_reconnects: u32, backoff: Duration) -> Self {
        self.max_idle_reconnects = max_idle_reconnects;
        self.reconnect_backoff = backoff;
        self
    }

    /// How often a terminal turn with null usage is read again before its
    /// usage is reported as unknown (tests shorten it).
    pub fn with_usage_poll(mut self, reads: u32, backoff: Duration) -> Self {
        self.usage_reads = reads.max(1);
        self.usage_backoff = backoff;
        self
    }

    /// Drive `request` to a terminal provider turn, resuming from whatever a
    /// previous owner of this session saved.
    pub async fn run(
        &self,
        request: &AgentsApiTurnRequest,
    ) -> Result<AgentsApiTurnOutcome, AgentsApiError> {
        let lease = AgentsApiLease {
            org_id: request.org_id,
            session_id: request.session_id,
            owner: Uuid::new_v4(),
        };
        let checkpoint = self.store.acquire(lease).await.map_err(store_error)?;
        let mut run = Run {
            driver: self,
            request,
            lease,
            checkpoint,
            deltas: HashMap::new(),
            stream_trips: HashMap::new(),
            last_error: None,
            terminal: false,
            session_failed: false,
            paused: false,
        };
        let outcome = {
            let execution = run.execute();
            tokio::pin!(execution);
            let cancelled = async {
                match self.cancellation.clone() {
                    Some(mut receiver) => loop {
                        if *receiver.borrow() || receiver.changed().await.is_err() {
                            break;
                        }
                    },
                    None => std::future::pending::<()>().await,
                }
            };
            let heartbeat = async {
                let mut interval = tokio::time::interval(self.heartbeat);
                interval.tick().await;
                loop {
                    interval.tick().await;
                    if let Err(error) = self.store.renew(lease).await {
                        break store_error(error);
                    }
                }
            };
            tokio::select! {
                outcome = &mut execution => outcome,
                _ = cancelled => Err(AgentsApiError::Cancelled),
                error = heartbeat => Err(error),
            }
        };
        match outcome {
            Err(AgentsApiError::Cancelled) => {
                run.cancel().await;
                Err(AgentsApiError::Cancelled)
            }
            Err(error) => run.settle_permanent_failure(error).await,
            outcome => outcome,
        }
    }
}

fn store_error(error: impl std::fmt::Display) -> AgentsApiError {
    AgentsApiError::Store(error.to_string())
}

/// What must already be in the event log for a `completing` record to count
/// as completed.
enum Recorded {
    Message(MessageId),
    ToolResult(String),
    /// An event the log cannot be searched for (accounting, hosted calls,
    /// reasoning summaries, compaction). After a crash between the save and
    /// the emit it counts as recorded: a lost record beats a doubled debit.
    Unverifiable,
}

struct Run<'a> {
    driver: &'a AgentsApiTurnDriver,
    request: &'a AgentsApiTurnRequest,
    lease: AgentsApiLease,
    checkpoint: AgentsApiCheckpoint,
    /// Live text per provider message item, for delta events only.
    deltas: HashMap<String, String>,
    /// Streaming guardrail trips per provider message item; the completed
    /// message is replaced.
    stream_trips: HashMap<String, TrippedGuardrail>,
    last_error: Option<(Option<String>, String)>,
    terminal: bool,
    /// The provider session (or its environment) failed: the turn ends even
    /// if the provider never marks the root turn terminal.
    session_failed: bool,
    /// A call of this turn is parked on a user action.
    paused: bool,
}

impl Run<'_> {
    fn turn(&self) -> &AgentsApiTurnCheckpoint {
        self.checkpoint
            .turn
            .as_ref()
            .expect("turn checkpoint initialized at start")
    }

    fn turn_mut(&mut self) -> &mut AgentsApiTurnCheckpoint {
        self.checkpoint
            .turn_mut(self.request.turn_id, self.request.input_message_id)
    }

    async fn save(&self) -> Result<(), AgentsApiError> {
        self.driver
            .store
            .save(self.lease, &self.checkpoint)
            .await
            .map_err(store_error)
    }

    fn provider_session(&self) -> Result<String, AgentsApiError> {
        self.checkpoint
            .provider_session_id
            .clone()
            .ok_or_else(|| AgentsApiError::Reconcile("no provider session".into()))
    }

    /// A parked call or a policy stop ends the event loop early.
    fn halted(&self) -> bool {
        self.paused || self.turn().policy_stop.is_some()
    }

    async fn execute(&mut self) -> Result<AgentsApiTurnOutcome, AgentsApiError> {
        self.cancel_abandoned_turn().await;
        self.turn_mut();
        if let Some(outcome) = self.turn().outcome.clone() {
            // Terminal recovery: the activity is replaying a finished turn.
            let outcome = serde_json::from_value(outcome).map_err(store_error)?;
            self.release().await?;
            return Ok(outcome);
        }
        if self.turn().policy_stop.is_some() {
            return self.finish_policy_stop().await;
        }
        let mut stream = self.start().await?;
        let mut idle_reconnects = 0;
        while !self.terminal && !self.halted() {
            match stream.next().await {
                Some(Ok(event)) => {
                    idle_reconnects = 0;
                    self.apply_event(&event).await?;
                }
                Some(Err(AgentsApiError::Http(error))) => {
                    tracing::warn!(error, "Agents API stream failed; reconciling");
                    stream = self.reconnect(&mut idle_reconnects).await?;
                }
                Some(Err(error)) => return Err(error),
                None => stream = self.reconnect(&mut idle_reconnects).await?,
            }
        }
        if self.turn().policy_stop.is_some() {
            return self.finish_policy_stop().await;
        }
        if self.paused {
            // The provider keeps the required action open. Nothing terminal
            // is saved: the resumed turn picks the parked call up again.
            self.release().await?;
            return Ok(AgentsApiTurnOutcome::Paused);
        }
        self.finalize().await
    }

    /// Ensure the provider session exists and holds this turn's input, and
    /// return a live event stream for it.
    async fn start(&mut self) -> Result<AgentsApiEventStream, AgentsApiError> {
        let fingerprint = self.request.config.agent_fingerprint();
        let staged = self.turn().input.is_some();
        if !staged
            && self.checkpoint.provider_session_id.is_some()
            && self.checkpoint.agent_fingerprint.as_deref() != Some(fingerprint.as_str())
        {
            // A provider session keeps the agent it was created with.
            tracing::warn!(
                session_id = %self.request.session_id,
                "Agents API agent definition changed; starting a new provider session"
            );
            self.checkpoint.release_provider_session();
        }
        if !staged
            && self.checkpoint.provider_session_id.is_some()
            && self.checkpoint.provider_key.is_some()
            && self.checkpoint.provider_key != self.request.provider_key
        {
            // The session belongs to the credentials that created it; another
            // provider (another OpenAI project) cannot reach it. A key rotated
            // on the same provider keeps the session.
            tracing::warn!(
                session_id = %self.request.session_id,
                "Agents API provider changed; starting a new provider session"
            );
            self.checkpoint.release_provider_session();
        }
        if self.checkpoint.provider_key.is_none() && self.checkpoint.provider_session_id.is_some() {
            // A checkpoint written before the provider was recorded: the
            // session was created with this turn's provider.
            self.checkpoint.provider_key = self.request.provider_key.clone();
        }
        if self.checkpoint.provider_session_id.is_none()
            && let Some(attempt) = self.checkpoint.create_attempt.clone()
            && let Some(session_id) = self.adopt_created_session(&attempt).await?
        {
            self.checkpoint.provider_session_id = Some(session_id);
            self.input_delivered();
            self.save().await?;
        }
        let Some(session_id) = self.checkpoint.provider_session_id.clone() else {
            return self.create_session(fingerprint).await;
        };
        if self.turn().input.is_none() {
            let prior = self.root_turn_ids(&session_id).await?;
            let key = format!("everruns-input-{}", self.request.input_message_id);
            let turn = self.turn_mut();
            turn.prior_provider_turns = prior;
            turn.input = Some(InputOutbox {
                idempotency_key: key,
                state: OutboxState::Pending,
            });
            self.save().await?;
        }
        // Open the stream before sending or reconciling so nothing that
        // happens afterwards is missed; reconciliation covers what came before.
        let stream = self
            .driver
            .client
            .stream_session_events(&session_id)
            .await?;
        let pending = self
            .turn()
            .input
            .as_ref()
            .filter(|input| input.state == OutboxState::Pending)
            .map(|input| input.idempotency_key.clone());
        if let Some(key) = pending
            && self.turn().provider_turn_id.is_none()
        {
            self.driver
                .client
                .send_events(
                    &session_id,
                    vec![build_message_input(&self.request.input_text)],
                    Some(&key),
                )
                .await?;
            self.input_delivered();
            self.save().await?;
        }
        self.reconcile().await?;
        Ok(stream)
    }

    async fn create_session(
        &mut self,
        fingerprint: String,
    ) -> Result<AgentsApiEventStream, AgentsApiError> {
        // Refused before anything is saved or sent.
        self.request.config.validate_direct_mcp()?;
        let attempt = Uuid::new_v4().to_string();
        self.checkpoint.create_attempt = Some(attempt.clone());
        self.checkpoint.agent_fingerprint = Some(fingerprint);
        self.checkpoint.provider_key = self.request.provider_key.clone();
        let turn = self.turn_mut();
        turn.prior_provider_turns.clear();
        turn.provider_turn_id = None;
        turn.input = Some(InputOutbox {
            idempotency_key: attempt.clone(),
            state: OutboxState::Pending,
        });
        self.save().await?;
        let mut config = self.request.config.clone();
        config.input = Value::String(self.request.input_text.clone());
        config.metadata.insert(
            METADATA_SESSION_KEY.to_string(),
            self.request.session_id.to_string(),
        );
        config
            .metadata
            .insert(METADATA_ATTEMPT_KEY.to_string(), attempt);
        self.driver.client.create_session_stream(&config).await
    }

    /// Find the session an uncertain create produced. Sessions list newest
    /// first, and the attempt id is unique, so one page suffices.
    async fn adopt_created_session(&self, attempt: &str) -> Result<Option<String>, AgentsApiError> {
        let session_key = self.request.session_id.to_string();
        Ok(self
            .driver
            .client
            .list_recent_sessions()
            .await?
            .into_iter()
            .find(|session| {
                session.pointer(&format!("/metadata/{METADATA_ATTEMPT_KEY}"))
                    == Some(&Value::String(attempt.to_string()))
                    && session.pointer(&format!("/metadata/{METADATA_SESSION_KEY}"))
                        == Some(&Value::String(session_key.clone()))
            })
            .and_then(|session| session.get("id")?.as_str().map(str::to_string)))
    }

    async fn root_turn_ids(&self, session_id: &str) -> Result<Vec<String>, AgentsApiError> {
        Ok(self
            .driver
            .client
            .list_turns(session_id)
            .await?
            .into_iter()
            .filter(|turn| turn.get("subagent_id").is_none_or(Value::is_null))
            .filter_map(|turn| turn.get("id")?.as_str().map(str::to_string))
            .collect())
    }

    fn input_delivered(&mut self) {
        if let Some(input) = self.turn_mut().input.as_mut() {
            input.state = OutboxState::Delivered;
        }
    }

    async fn reconnect(
        &mut self,
        idle_reconnects: &mut u32,
    ) -> Result<AgentsApiEventStream, AgentsApiError> {
        *idle_reconnects += 1;
        if *idle_reconnects > self.driver.max_idle_reconnects {
            return Err(AgentsApiError::StreamClosedBeforeTurnEnded);
        }
        tokio::time::sleep(self.driver.reconnect_backoff * (*idle_reconnects - 1)).await;
        if self.checkpoint.provider_session_id.is_none() {
            // The create stream ended before it named the session.
            return self.start().await;
        }
        let session_id = self.provider_session()?;
        let stream = self
            .driver
            .client
            .stream_session_events(&session_id)
            .await?;
        self.reconcile().await?;
        Ok(stream)
    }

    /// Bring the checkpoint and the event log up to the provider's saved state.
    async fn reconcile(&mut self) -> Result<(), AgentsApiError> {
        let session_id = self.provider_session()?;
        let session = self.driver.client.retrieve_session(&session_id).await?;
        if self.turn().provider_turn_id.is_none() {
            let prior = &self.turn().prior_provider_turns;
            let mut candidates = self
                .driver
                .client
                .list_turns(&session_id)
                .await?
                .into_iter()
                .filter(|turn| turn.get("subagent_id").is_none_or(Value::is_null))
                .filter(|turn| {
                    turn.get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| !prior.iter().any(|known| known == id))
                })
                .collect::<Vec<_>>();
            candidates.sort_by_key(|turn| turn.get("created_at").and_then(Value::as_u64));
            if let Some(id) = candidates
                .first()
                .and_then(|turn| turn.get("id")?.as_str().map(str::to_string))
            {
                self.adopt_turn(id).await?;
            }
        }
        let Some(turn_id) = self.turn().provider_turn_id.clone() else {
            if is_failed_status(&session) {
                self.record_failure(session.get("error"));
                self.terminal = true;
            }
            return Ok(());
        };
        for item in self
            .driver
            .client
            .list_turn_items(&session_id, &turn_id)
            .await?
        {
            if self.turn().policy_stop.is_some() {
                return Ok(());
            }
            self.apply_item(&item).await?;
        }
        self.handle_actions(FunctionCallAction::from_required_actions(&session))
            .await?;
        let turn = self
            .driver
            .client
            .retrieve_turn(&session_id, &turn_id)
            .await?;
        if is_terminal_status(&turn) {
            self.terminal = true;
        }
        Ok(())
    }

    async fn adopt_turn(&mut self, provider_turn_id: String) -> Result<(), AgentsApiError> {
        self.turn_mut().provider_turn_id = Some(provider_turn_id);
        self.input_delivered();
        self.save().await
    }

    async fn apply_event(&mut self, event: &Value) -> Result<(), AgentsApiError> {
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .ok_or(AgentsApiError::MissingEventType)?;
        if self.checkpoint.provider_session_id.is_none()
            && let Some(id) = event
                .pointer("/session/id")
                .or_else(|| event.get("session_id"))
                .and_then(Value::as_str)
        {
            // Save the session before applying anything that belongs to it.
            self.checkpoint.provider_session_id = Some(id.to_string());
            self.input_delivered();
            self.save().await?;
        }
        if is_subagent_event(event) {
            // A subagent's own events never end, or write into, the root
            // turn; only its lifecycle is recorded.
            return self.apply_subagent_event(event_type, event).await;
        }
        let event_turn = event
            .get("turn_id")
            .or_else(|| event.pointer("/turn/id"))
            .and_then(Value::as_str);
        if event_type == "agent.session.turn.created"
            && self.turn().provider_turn_id.is_none()
            && let Some(id) = event_turn
            && !self
                .turn()
                .prior_provider_turns
                .iter()
                .any(|known| known == id)
        {
            self.adopt_turn(id.to_string()).await?;
        }
        let own_turn = self.turn().provider_turn_id.as_deref();
        if event_turn.is_some() && (own_turn.is_none() || event_turn != own_turn) {
            return Ok(());
        }
        if is_turn_terminal_event(event_type) && event_turn.is_none() {
            // A turn terminal that names no turn cannot be attributed to this
            // one; the turn resource decides when reconciliation reads it.
            tracing::warn!(
                event_type,
                "Agents API: ignoring a turn terminal event without a turn id"
            );
            return Ok(());
        }
        if let Some(id) = event.get("event_id").and_then(Value::as_str) {
            self.turn_mut().last_event_id = Some(id.to_string());
        }
        match event_type {
            "agent.session.turn.output_text.delta" => {
                let (Some(item_id), Some(delta)) = (
                    event.get("item_id").and_then(Value::as_str),
                    event.get("delta").and_then(Value::as_str),
                ) else {
                    return Ok(());
                };
                self.emit_delta(item_id, delta).await?;
            }
            "agent.session.turn.item.added"
            | "agent.session.turn.item.updated"
            | "agent.session.turn.item.done" => {
                if let Some(item) = event.get("item") {
                    self.apply_item(item).await?;
                }
            }
            "agent.session.requires_action" => {
                self.handle_actions(FunctionCallAction::from_required_actions(event))
                    .await?;
            }
            "error" => {
                // The live API reports the cause (e.g. `usage_limit_exceeded`)
                // on a standalone `error` event just before `turn.failed`.
                self.record_failure(event.get("error"));
            }
            "agent.session.turn.completed"
            | "agent.session.turn.failed"
            | "agent.session.turn.cancelled" => {
                if event_type == "agent.session.turn.failed" {
                    self.record_failure(event.pointer("/turn/error"));
                }
                self.terminal = true;
            }
            "agent.session.failed" | "agent.session.environment.failed" => {
                self.record_failure(
                    event
                        .pointer("/session/error")
                        .filter(|error| !error.is_null())
                        .or_else(|| event.get("error")),
                );
                if self.last_error.is_none() {
                    self.last_error = Some(if event_type == "agent.session.environment.failed" {
                        (
                            Some("environment_failed".to_string()),
                            "The OpenAI Agents API environment failed".to_string(),
                        )
                    } else {
                        (
                            Some("session_failed".to_string()),
                            "The OpenAI Agents API session failed".to_string(),
                        )
                    });
                }
                self.session_failed = true;
                self.terminal = true;
            }
            other if other.ends_with(".failed") || other.ends_with(".cancelled") => {
                return Err(AgentsApiError::UnsupportedTerminalEvent(other.to_string()));
            }
            _ => {}
        }
        Ok(())
    }

    fn record_failure(&mut self, error: Option<&Value>) {
        let Some(error) = error.filter(|error| !error.is_null()) else {
            return;
        };
        let code = error
            .get("code")
            .and_then(Value::as_str)
            .map(str::to_string);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| self.last_error.as_ref().map(|(_, message)| message.clone()))
            .unwrap_or_else(|| "OpenAI Agents API error".to_string());
        self.last_error = Some((code.or_else(|| self.last_error.clone()?.0), message));
    }

    /// Provider correlation beside the local ids (see
    /// `everruns_core::events::correlation`).
    fn metadata(&self, item_id: Option<&str>) -> Value {
        let session = self.checkpoint.provider_session_id.as_deref();
        json!({
            RUNTIME_BACKEND: "openai_agents_api",
            PROVIDER_SESSION_ID: session,
            PROVIDER_TURN_ID: self.turn().provider_turn_id,
            PROVIDER_ITEM_ID: item_id,
            PROVIDER_TRACE_URL: session.and_then(|id| self.driver.client.trace_url(id)),
        })
    }

    fn event(
        &self,
        item_id: Option<&str>,
        data: impl Into<everruns_core::events::EventData>,
    ) -> EventRequest {
        let mut event = EventRequest::new(
            self.request.session_id,
            self.request.event_context.clone(),
            data,
        );
        event.metadata = Some(self.metadata(item_id));
        event
    }

    async fn emit(&self, events: Vec<EventRequest>) -> Result<(), AgentsApiError> {
        for event in events {
            self.driver.ledger.emit(event).await?;
        }
        Ok(())
    }

    /// Record a correlation as open, emitting its start events at most once.
    /// Returns the local id the item maps to.
    async fn open(
        &mut self,
        key: &str,
        kind: ItemKind,
        new_local_id: impl FnOnce() -> String,
        start_events: impl FnOnce(&Self, &str) -> Vec<EventRequest>,
    ) -> Result<String, AgentsApiError> {
        if let Some(existing) = self.turn().items.get(key) {
            return Ok(existing.local_id.clone());
        }
        let local_id = new_local_id();
        self.turn_mut().items.insert(
            key.to_string(),
            ItemCorrelation {
                kind,
                local_id: local_id.clone(),
                state: ItemState::Open,
            },
        );
        self.save().await?;
        let events = start_events(self, &local_id);
        self.emit(events).await?;
        Ok(local_id)
    }

    /// Emit an item's completing events exactly once across restarts.
    async fn complete(
        &mut self,
        key: &str,
        kind: ItemKind,
        local_id: &str,
        recorded: Recorded,
        events: Vec<EventRequest>,
    ) -> Result<(), AgentsApiError> {
        match self.turn().items.get(key).map(|item| item.state) {
            Some(ItemState::Completed) => return Ok(()),
            Some(ItemState::Completing) if self.is_recorded(&recorded).await? => {}
            _ => {
                self.set_item_state(key, kind, local_id, ItemState::Completing);
                self.save().await?;
                self.emit(events).await?;
            }
        }
        self.set_item_state(key, kind, local_id, ItemState::Completed);
        self.save().await
    }

    fn set_item_state(&mut self, key: &str, kind: ItemKind, local_id: &str, state: ItemState) {
        self.turn_mut()
            .items
            .entry(key.to_string())
            .and_modify(|item| item.state = state)
            .or_insert_with(|| ItemCorrelation {
                kind,
                local_id: local_id.to_string(),
                state,
            });
    }

    async fn is_recorded(&self, recorded: &Recorded) -> Result<bool, AgentsApiError> {
        let session_id = self.request.session_id;
        match recorded {
            Recorded::Message(id) => self.driver.ledger.has_message(session_id, *id).await,
            Recorded::ToolResult(call_id) => Ok(self
                .driver
                .ledger
                .tool_result(session_id, call_id)
                .await?
                .is_some()),
            Recorded::Unverifiable => Ok(true),
        }
    }

    async fn emit_delta(&mut self, item_id: &str, delta: &str) -> Result<(), AgentsApiError> {
        let key = format!("msg:{item_id}");
        let Some(item) = self.turn().items.get(&key) else {
            return Ok(());
        };
        if item.state != ItemState::Open {
            return Ok(());
        }
        let message_id = parse_message_id(&item.local_id)?;
        if self.stream_trips.contains_key(item_id) {
            return Ok(());
        }
        let accumulated = self.deltas.entry(item_id.to_string()).or_default();
        accumulated.push_str(delta);
        let accumulated = accumulated.clone();
        if let Some(policy) = &self.driver.output_policy {
            if let Some(trip) = policy.check_delta(item_id, &accumulated, delta) {
                // Nothing more of this message reaches the client; the
                // completed item is replaced.
                self.stream_trips.insert(item_id.to_string(), trip);
                return Ok(());
            }
            if policy.withholds_deltas() {
                return Ok(());
            }
        }
        let event = self.event(
            Some(item_id),
            OutputMessageDeltaData {
                turn_id: self.request.turn_id,
                message_id,
                delta: delta.to_string(),
                accumulated,
                phase: None,
            },
        );
        self.driver.ledger.emit(event).await
    }

    fn model(&self) -> String {
        self.request.config.agent.model.clone()
    }

    async fn apply_item(&mut self, item: &Value) -> Result<(), AgentsApiError> {
        let Some(item_id) = item.get("id").and_then(Value::as_str) else {
            return Ok(());
        };
        if let (Some(own), Some(item_turn)) = (
            self.turn().provider_turn_id.as_deref(),
            item.get("turn_id").and_then(Value::as_str),
        ) && own != item_turn
        {
            return Ok(());
        }
        let status = item.get("status").and_then(Value::as_str);
        match item.get("type").and_then(Value::as_str) {
            Some("message") if item.get("role").and_then(Value::as_str) == Some("assistant") => {
                self.apply_message(item_id, item, status).await
            }
            Some("function_call") => {
                let call_id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .unwrap_or(item_id);
                let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
                self.record_function_call(call_id, name, &super::arguments_value(item))
                    .await
            }
            Some("function_call_output") => {
                let Some(call_id) = item.get("call_id").and_then(Value::as_str) else {
                    return Ok(());
                };
                // The provider holds our output: the submission landed.
                let mut changed = false;
                if let Some(entry) = self.turn_mut().tool_results.get_mut(call_id)
                    && let ToolResultState::Ready { success, output } = &entry.state
                {
                    entry.state = ToolResultState::Submitted {
                        success: *success,
                        output: output.clone(),
                    };
                    changed = true;
                }
                if changed {
                    self.save().await?;
                }
                Ok(())
            }
            Some("mcp_call") => self.apply_mcp_call(item_id, item, status).await,
            Some("reasoning") => self.apply_reasoning(item_id, item, status).await,
            Some("compaction") => self.apply_compaction(item_id, item, status).await,
            Some(kind) if observe::is_hosted_call(kind) => {
                self.apply_hosted_call(item_id, kind, item, status).await
            }
            // Unknown item kinds (and user input echoes) are not projected.
            _ => Ok(()),
        }
    }

    async fn apply_message(
        &mut self,
        item_id: &str,
        item: &Value,
        status: Option<&str>,
    ) -> Result<(), AgentsApiError> {
        let key = format!("msg:{item_id}");
        if self.turn().items.get(&key).map(|item| item.state) == Some(ItemState::Completed) {
            return Ok(());
        }
        let phase = match item.get("phase").and_then(Value::as_str) {
            Some("commentary") => Some(ExecutionPhase::Commentary),
            Some("final_answer") => Some(ExecutionPhase::FinalAnswer),
            _ => None,
        };
        let model = self.model();
        let local_id = self
            .open(
                &key,
                ItemKind::Message,
                || MessageId::new().to_string(),
                |run, local_id| {
                    let Ok(message_id) = parse_message_id(local_id) else {
                        return Vec::new();
                    };
                    vec![run.event(
                        Some(item_id),
                        OutputMessageStartedData {
                            reasoning_state: None,
                            turn_id: run.request.turn_id,
                            message_id,
                            model: Some(model.clone()),
                            iteration: Some(1),
                            phase,
                        },
                    )]
                },
            )
            .await?;
        if status != Some("completed") {
            return Ok(());
        }
        let message_id = parse_message_id(&local_id)?;
        let mut text = message_item_text(item);
        if text.is_empty() {
            text = self.deltas.get(item_id).cloned().unwrap_or_default();
        }
        // THREAT[TM-LLM-043]: Everruns output guardrails run on every remote
        // assistant message before it is recorded.
        let trip = match self.stream_trips.remove(item_id) {
            Some(trip) => Some(trip),
            None => match &self.driver.output_policy {
                Some(policy) => policy.check_message(item_id, &text).await,
                None => None,
            },
        };
        if let Some(trip) = trip {
            tracing::info!(
                session_id = %self.request.session_id,
                guardrail_capability_id = %trip.capability_id,
                guardrail_id = %trip.guardrail_id,
                "Agents API: output guardrail tripped on a remote message"
            );
            // Saved before any effect, so a replay finishes the stop instead
            // of re-judging the message.
            self.set_item_state(&key, ItemKind::Message, &local_id, ItemState::Completing);
            self.turn_mut().policy_stop = Some(PolicyStop {
                code: OUTPUT_GUARDRAIL_STOP.to_string(),
                message: trip.block.replacement,
                replaced: Some(ReplacedMessage {
                    item_key: key,
                    provider_item_id: item_id.to_string(),
                    message_id,
                    guardrail_capability_id: trip.capability_id,
                    guardrail_id: trip.guardrail_id,
                    reason_code: trip.block.reason_code,
                }),
            });
            return self.save().await;
        }
        let mut message = RuntimeMessage::assistant(text).with_id(message_id);
        if let Some(phase) = phase {
            message = message.with_phase(phase);
        }
        let event = self.event(
            Some(item_id),
            OutputMessageCompletedData::new(message).with_metadata(ModelMetadata {
                model,
                model_id: None,
                provider_id: None,
            }),
        );
        self.complete(
            &key,
            ItemKind::Message,
            &local_id,
            Recorded::Message(message_id),
            vec![event],
        )
        .await
    }

    /// Record a client function call as an assistant tool-call message, as
    /// the native runtime does, so the transcript stays replayable.
    async fn record_function_call(
        &mut self,
        call_id: &str,
        name: &str,
        arguments: &Value,
    ) -> Result<(), AgentsApiError> {
        let key = format!("call:{call_id}");
        if self.turn().items.get(&key).map(|item| item.state) == Some(ItemState::Completed) {
            return Ok(());
        }
        let local_id = match self.turn().items.get(&key) {
            Some(item) => item.local_id.clone(),
            None => MessageId::new().to_string(),
        };
        let message_id = parse_message_id(&local_id)?;
        let message = RuntimeMessage::assistant_with_tools(
            "",
            vec![ToolCall {
                id: call_id.to_string(),
                name: name.to_string(),
                arguments: arguments.clone(),
            }],
        )
        .with_id(message_id);
        let event = self.event(
            Some(call_id),
            OutputMessageCompletedData::new(message).with_metadata(ModelMetadata {
                model: self.model(),
                model_id: None,
                provider_id: None,
            }),
        );
        self.complete(
            &key,
            ItemKind::FunctionCall,
            &local_id,
            Recorded::Message(message_id),
            vec![event],
        )
        .await
    }

    async fn apply_mcp_call(
        &mut self,
        item_id: &str,
        item: &Value,
        status: Option<&str>,
    ) -> Result<(), AgentsApiError> {
        let server_label = item
            .get("server_label")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let source_name = item.get("name").and_then(Value::as_str).unwrap_or_default();
        let tool_name = mcp_tool_name(server_label, source_name);
        let display_name = format!("{server_label}: {source_name}");
        let call = ToolCall {
            id: item_id.to_string(),
            name: tool_name.clone(),
            arguments: super::arguments_value(item),
        };
        let call_key = format!("mcp:{item_id}");
        if self.turn().items.get(&call_key).map(|item| item.state) != Some(ItemState::Completed) {
            let local_id = match self.turn().items.get(&call_key) {
                Some(item) => item.local_id.clone(),
                None => MessageId::new().to_string(),
            };
            let message_id = parse_message_id(&local_id)?;
            let message =
                RuntimeMessage::assistant_with_tools("", vec![call.clone()]).with_id(message_id);
            let events = vec![
                self.event(
                    Some(item_id),
                    OutputMessageCompletedData::new(message).with_metadata(ModelMetadata {
                        model: self.model(),
                        model_id: None,
                        provider_id: None,
                    }),
                ),
                self.event(
                    Some(item_id),
                    ToolStartedData {
                        tool_call: call.clone(),
                        tool_call_fingerprint: None,
                        display_name: Some(display_name.clone()),
                        narration: None,
                    },
                ),
            ];
            self.complete(
                &call_key,
                ItemKind::McpCall,
                &local_id,
                Recorded::Message(message_id),
                events,
            )
            .await?;
        }
        let error = item.get("error").filter(|error| !error.is_null());
        let failed = status == Some("failed") || error.is_some();
        if !failed && status != Some("completed") {
            return Ok(());
        }
        let duration_ms = item.get("duration_ms").and_then(Value::as_u64);
        let data = if failed {
            // A failed call reports its cause in `output`, with `error` null.
            let message = error
                .and_then(|e| e.get("message").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_else(|| provider_output_text(item));
            ToolCompletedData::failure(
                item_id.to_string(),
                tool_name,
                "error".to_string(),
                message,
                duration_ms,
            )
        } else {
            ToolCompletedData::success(
                item_id.to_string(),
                tool_name,
                vec![ContentPart::text(provider_output_text(item))],
                duration_ms,
            )
        }
        .with_display_name(Some(display_name));
        let event = self.event(Some(item_id), data);
        self.complete(
            &format!("mcp-result:{item_id}"),
            ItemKind::McpCall,
            item_id,
            Recorded::ToolResult(item_id.to_string()),
            vec![event],
        )
        .await
    }

    /// Reconcile once more, then save and return the outcome.
    async fn finalize(&mut self) -> Result<AgentsApiTurnOutcome, AgentsApiError> {
        let outcome = match self.turn().provider_turn_id.clone() {
            Some(turn_id) => {
                let session_id = self.provider_session()?;
                let items = self
                    .driver
                    .client
                    .list_turn_items(&session_id, &turn_id)
                    .await?;
                for item in &items {
                    self.apply_item(item).await?;
                }
                let turn = self
                    .driver
                    .client
                    .retrieve_turn(&session_id, &turn_id)
                    .await?;
                let mut outcome = self.outcome_from(&turn, &items)?;
                let final_text = match &outcome {
                    AgentsApiTurnOutcome::Completed { final_text, .. } => Some(final_text.clone()),
                    _ => None,
                };
                let usage = self.account(Some(turn), final_text, true).await?;
                if let AgentsApiTurnOutcome::Completed {
                    usage: reported, ..
                } = &mut outcome
                {
                    *reported = usage;
                }
                outcome
            }
            // The session failed before a root turn started.
            None => {
                let (code, message) = self.last_error.clone().unwrap_or((
                    None,
                    "OpenAI Agents API session failed before the turn started".to_string(),
                ));
                AgentsApiTurnOutcome::Failed {
                    code,
                    message,
                    policy: false,
                }
            }
        };
        self.turn_mut().outcome = Some(serde_json::to_value(&outcome).map_err(store_error)?);
        self.save().await?;
        self.release().await?;
        Ok(outcome)
    }

    /// Calls still parked when the provider turn ended.
    fn parked_calls(&self) -> usize {
        self.turn()
            .tool_results
            .values()
            .filter(|entry| matches!(entry.state, ToolResultState::Parked { .. }))
            .count()
    }

    fn outcome_from(
        &self,
        turn: &Value,
        items: &[Value],
    ) -> Result<AgentsApiTurnOutcome, AgentsApiError> {
        match turn.get("status").and_then(Value::as_str) {
            Some("completed") => {
                let final_item = items.iter().rev().find(|item| {
                    item.get("type").and_then(Value::as_str) == Some("message")
                        && item.get("role").and_then(Value::as_str) == Some("assistant")
                        && item.get("phase").and_then(Value::as_str) != Some("commentary")
                });
                let final_message_id = final_item
                    .and_then(|item| item.get("id")?.as_str())
                    .and_then(|id| self.turn().items.get(&format!("msg:{id}")))
                    .map(|item| parse_message_id(&item.local_id))
                    .transpose()?;
                Ok(AgentsApiTurnOutcome::Completed {
                    final_message_id,
                    final_text: final_item.map(message_item_text).unwrap_or_default(),
                    usage: usage_from(turn),
                    tool_calls: self.tool_call_count(),
                })
            }
            // The provider gave up on the turn while Everruns held a call
            // open for a person (its own timeout, or a cancel elsewhere).
            Some("failed" | "cancelled") if self.parked_calls() > 0 => {
                let cause = turn
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(|message| format!(" Provider: {message}"))
                    .unwrap_or_default();
                Ok(AgentsApiTurnOutcome::Failed {
                    code: Some(PARKED_CALL_EXPIRED.to_string()),
                    message: format!(
                        "The OpenAI Agents API turn ended while waiting for an answer to {} tool call(s); start a new turn to continue.{cause}",
                        self.parked_calls()
                    ),
                    policy: true,
                })
            }
            Some("failed") => {
                let error = turn.get("error").filter(|error| !error.is_null());
                let fallback = self.last_error.clone();
                Ok(AgentsApiTurnOutcome::Failed {
                    code: error
                        .and_then(|e| e.get("code")?.as_str().map(str::to_string))
                        .or_else(|| fallback.as_ref()?.0.clone()),
                    message: error
                        .and_then(|e| e.get("message")?.as_str().map(str::to_string))
                        .or_else(|| fallback.map(|(_, message)| message))
                        .unwrap_or_else(|| "OpenAI Agents API turn failed".to_string()),
                    policy: false,
                })
            }
            Some("cancelled") => Ok(AgentsApiTurnOutcome::Cancelled),
            // The session or its environment failed under a turn the
            // provider never closed.
            _ if self.session_failed => {
                let (code, message) = self
                    .last_error
                    .clone()
                    .unwrap_or((None, "OpenAI Agents API session failed".to_string()));
                Ok(AgentsApiTurnOutcome::Failed {
                    code,
                    message,
                    policy: false,
                })
            }
            other => Err(AgentsApiError::Reconcile(format!(
                "root turn is not terminal (status {other:?})"
            ))),
        }
    }

    async fn release(&self) -> Result<(), AgentsApiError> {
        self.driver
            .store
            .release(self.lease)
            .await
            .map_err(store_error)
    }

    /// A provider failure no retry fixes ends the turn with a stable code
    /// (see [`super::lifecycle`]); anything else stays an error for the
    /// durable engine to retry. A provider session that no longer exists is
    /// released, so the next turn starts a new one instead of failing again;
    /// the store queues the dropped id for deletion, which finds it gone.
    async fn settle_permanent_failure(
        &mut self,
        error: AgentsApiError,
    ) -> Result<AgentsApiTurnOutcome, AgentsApiError> {
        let has_session = self.checkpoint.provider_session_id.is_some();
        let Some(failure) = super::lifecycle::classify(&error, has_session) else {
            return Err(error);
        };
        if failure.code == super::lifecycle::PROVIDER_SESSION_UNAVAILABLE {
            // A 404 may name a turn or an item; only a missing session
            // releases the session.
            let session_id = self.provider_session()?;
            match self.driver.client.retrieve_session(&session_id).await {
                Err(AgentsApiError::Api { status: 404, .. }) => {}
                _ => return Err(error),
            }
            tracing::warn!(
                session_id = %self.request.session_id,
                "Agents API provider session no longer exists; releasing it"
            );
            self.checkpoint.release_provider_session();
        } else {
            tracing::warn!(
                session_id = %self.request.session_id,
                code = failure.code,
                "Agents API turn failed permanently"
            );
        }
        let outcome = AgentsApiTurnOutcome::Failed {
            code: Some(failure.code.to_string()),
            message: failure.message,
            policy: false,
        };
        self.turn_mut().outcome = Some(serde_json::to_value(&outcome).map_err(store_error)?);
        self.save().await?;
        self.release().await?;
        Ok(outcome)
    }

    /// Best effort: stop the provider turn and keep the cancellation durable
    /// so a replayed activity does not restart it.
    async fn cancel(&mut self) {
        if let Some(session_id) = self.checkpoint.provider_session_id.clone()
            && let Err(error) = self
                .driver
                .client
                .send_events(
                    &session_id,
                    vec![json!({"type": "agent.session.input.cancel"})],
                    None,
                )
                .await
        {
            tracing::warn!(%error, "Agents API cancel failed");
        }
        // Whatever the cancelled turn spent is still billed; usage the
        // provider has not reported yet is recorded as an unknown amount.
        if self
            .checkpoint
            .turn
            .as_ref()
            .is_some_and(|turn| turn.turn_id == self.request.turn_id)
            && let Err(error) = self.account(None, None, false).await
        {
            tracing::warn!(%error, "Agents API: accounting a cancelled turn failed");
        }
        if let Ok(outcome) = serde_json::to_value(AgentsApiTurnOutcome::Cancelled) {
            self.turn_mut().outcome = Some(outcome);
        }
        if self.save().await.is_ok() {
            let _ = self.release().await;
        }
    }
}

fn parse_message_id(raw: &str) -> Result<MessageId, AgentsApiError> {
    raw.parse()
        .map_err(|_| AgentsApiError::Store(format!("invalid local message id '{raw}'")))
}

fn is_terminal_status(resource: &Value) -> bool {
    matches!(
        resource.get("status").and_then(Value::as_str),
        Some("completed" | "failed" | "cancelled")
    )
}

fn is_turn_terminal_event(event_type: &str) -> bool {
    matches!(
        event_type,
        "agent.session.turn.completed"
            | "agent.session.turn.failed"
            | "agent.session.turn.cancelled"
    )
}

fn is_failed_status(resource: &Value) -> bool {
    resource.get("status").and_then(Value::as_str) == Some("failed")
}
