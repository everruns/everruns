//! Steering: user input that joins a running turn at its next reason
//! boundary, and the in-process [`TurnInbox`] the generic turn loop takes it
//! through.
//!
//! Decision: the in-process inbox is the turn's [`TurnSteering`] plus the
//! session wake queue plus the in-memory parked turns. Wakes are counted at
//! the end of every reason but only taken at the start of the next one, and
//! a pending wake keeps steering from being taken or closed, exactly as the
//! loop did before it was host-generic.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::tool_types::ToolCall;
use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};

use super::{AcceptedTurnInput, InProcessRuntime, message_from_input};
use crate::InputMessage;
use crate::engine::TurnState;
use crate::event_emitter::EventEmitter;
use crate::events::{EventContext, EventRequest, InputMessageData};
use crate::host::turn_loop::{PendingInput, TurnInbox};

/// Concurrency-safe ingress for messages sent while an in-process turn runs.
///
/// Closing and observing an empty queue is one atomic operation. A sender can
/// therefore never be told that it steered a turn after that turn committed to
/// completion; rejected input belongs to the next turn instead.
///
/// This type, [`AcceptedTurnInput`], [`TurnSteeringPushError`],
/// [`InProcessRuntime::run_steerable_turn`] and
/// [`InProcessRuntime::append_accepted_inputs`] form the steering surface the
/// separately published `everruns` facade drives. It was `#[doc(hidden)]`
/// while that was treated as internal plumbing, which is what let a signature
/// change ship under a host patch release and break the published facade
/// (everruns/yolop#665) -- `#[doc(hidden)]` hides an item from rustdoc and from
/// `cargo-semver-checks`, but it does not make it private, and a contract
/// crossing a crates.io boundary is public whatever it is annotated with.
/// Change these together with a host minor bump and a facade release.
#[derive(Clone, Debug)]
pub struct TurnSteering {
    state: Arc<Mutex<TurnSteeringState>>,
}

/// Why a steered message could not join the running turn.
///
/// Either way the input is handed back so the caller can requeue it; see the
/// note on [`TurnSteering`].
#[derive(Debug)]
pub enum TurnSteeringPushError {
    /// The turn already committed to completion; the input belongs to the next one.
    Closed(Box<AcceptedTurnInput>),
    /// The steering queue is at capacity and is rejecting overflow.
    Full(Box<AcceptedTurnInput>),
}

/// Bounds user input retained between reason boundaries when a model or tool is slow.
// THREAT[TM-DOS-036]: reject overflow before accepting more steering input.
const TURN_STEERING_CAPACITY: usize = 256;

#[derive(Debug, Default)]
struct TurnSteeringState {
    open: bool,
    inputs: VecDeque<AcceptedTurnInput>,
}

impl TurnSteering {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(TurnSteeringState {
                open: true,
                inputs: VecDeque::new(),
            })),
        }
    }

    pub fn try_push(
        &self,
        input: AcceptedTurnInput,
    ) -> std::result::Result<(), TurnSteeringPushError> {
        let mut state = self.state.lock().expect("turn steering lock poisoned");
        if !state.open {
            return Err(TurnSteeringPushError::Closed(Box::new(input)));
        }
        if state.inputs.len() >= TURN_STEERING_CAPACITY {
            return Err(TurnSteeringPushError::Full(Box::new(input)));
        }
        state.inputs.push_back(input);
        Ok(())
    }

    /// Take every input accepted so far, leaving the ingress open.
    ///
    /// A host calls this at a reason boundary, before the reason runs, to
    /// deliver steering into that reason. Durable backends drive their own
    /// boundaries, so this is public for them as well.
    pub fn drain(&self) -> Vec<AcceptedTurnInput> {
        let mut state = self.state.lock().expect("turn steering lock poisoned");
        state.inputs.drain(..).collect()
    }

    /// Drain accepted input, or close the ingress when there is none.
    ///
    /// The one atomic decision a turn makes when it would otherwise finish:
    /// continue with the drained input, or commit to completion so every
    /// later push is rejected and belongs to the next turn.
    pub fn drain_or_close(&self) -> Vec<AcceptedTurnInput> {
        let mut state = self.state.lock().expect("turn steering lock poisoned");
        if state.inputs.is_empty() {
            state.open = false;
            return vec![];
        }
        state.inputs.drain(..).collect()
    }

    pub fn close(&self) {
        self.state.lock().expect("turn steering lock poisoned").open = false;
    }

    pub fn close_and_drain(&self) -> Vec<AcceptedTurnInput> {
        let mut state = self.state.lock().expect("turn steering lock poisoned");
        state.open = false;
        state.inputs.drain(..).collect()
    }
}

impl Default for TurnSteering {
    fn default() -> Self {
        Self::new()
    }
}

impl InProcessRuntime {
    /// Persist one accepted input as the session's canonical `input.message`
    /// event, exactly as a turn does when the input starts it or steers it at
    /// a reason boundary. Returns the message's id.
    ///
    /// Unlike [`append_accepted_inputs`](Self::append_accepted_inputs), this
    /// runs no `user_prompt_submit` hook: the caller passes the id to the next
    /// reason as a prompt message, which applies the hooks there. A durable
    /// backend that drives this runtime's turn steps from its own queue uses
    /// it to persist input the way this runtime's own turns do.
    pub async fn persist_accepted_input(
        &self,
        session_id: SessionId,
        input: AcceptedTurnInput,
    ) -> Result<MessageId> {
        let message = input.into_message();
        let message_id = message.id;
        self.event_emitter
            .emit(EventRequest::new(
                session_id,
                EventContext::empty(),
                InputMessageData::new(message),
            ))
            .await?;
        Ok(message_id)
    }

    async fn inject_steering_inputs(
        &self,
        session_id: SessionId,
        inputs: Vec<AcceptedTurnInput>,
    ) -> Result<Vec<MessageId>> {
        let mut message_ids = Vec::with_capacity(inputs.len());
        for input in inputs {
            message_ids.push(self.persist_accepted_input(session_id, input).await?);
        }
        Ok(message_ids)
    }

    /// Drain any queued task wakes for `session_id` and inject them into the
    /// conversation as user messages so the next reason reacts to them
    /// (EVE-681, part A). Returns the ids of the injected messages so prompt
    /// hooks can inspect them before the next provider call.
    ///
    /// Called at the top of every reason iteration — before the LLM call — so a
    /// task completion that landed during the previous act (or while idle) is
    /// visible to the very next iteration. `SessionWakeQueue::drain` is the
    /// exactly-once claim point: a drained wake is removed and never delivered
    /// twice, so a wake is delivered mid-turn XOR on the next turn's first
    /// drain, never both.
    async fn drain_and_inject_wakes(&self, session_id: SessionId) -> Result<Vec<MessageId>> {
        let Some(queue) = &self.session_wake_queue else {
            return Ok(vec![]);
        };
        let wakes = queue.drain(session_id);
        if wakes.is_empty() {
            return Ok(vec![]);
        }
        let mut message_ids = Vec::with_capacity(wakes.len());
        for wake in wakes {
            // Append one canonical input event; history is reconstructed from it.
            let message = message_from_input(InputMessage::user(wake.text));
            message_ids.push(message.id);
            self.event_emitter
                .emit(EventRequest::new(
                    session_id,
                    EventContext::empty(),
                    InputMessageData::new(message),
                ))
                .await?;
        }
        Ok(message_ids)
    }
}

/// The in-process [`TurnInbox`]: one turn's steering, the session's wake
/// queue, and this runtime's parked turns.
pub(super) struct InProcessInbox<'a> {
    runtime: &'a InProcessRuntime,
    steering: TurnSteering,
}

impl<'a> InProcessInbox<'a> {
    pub(super) fn new(runtime: &'a InProcessRuntime, steering: TurnSteering) -> Self {
        Self { runtime, steering }
    }
}

#[async_trait]
impl TurnInbox for InProcessInbox<'_> {
    async fn take(&self, session_id: SessionId) -> Result<Vec<MessageId>> {
        let mut message_ids = self
            .runtime
            .inject_steering_inputs(session_id, self.steering.drain())
            .await?;
        // Draining wakes here also delivers wakes that arrived while the
        // session was idle, on the next turn's first iteration (between-turn
        // fallback).
        message_ids.extend(self.runtime.drain_and_inject_wakes(session_id).await?);
        Ok(message_ids)
    }

    async fn after_reason(
        &self,
        session_id: SessionId,
        would_finish: bool,
    ) -> Result<PendingInput> {
        // A pending wake is delivered by the next `take`, so it neither takes
        // nor closes steering here.
        let pending_wakes = usize::from(
            self.runtime
                .session_wake_queue
                .as_ref()
                .is_some_and(|queue| queue.has_pending(session_id)),
        );
        let steering = if would_finish && pending_wakes == 0 {
            self.steering.drain_or_close()
        } else {
            vec![]
        };
        let count = pending_wakes + steering.len();
        let message_ids = self
            .runtime
            .inject_steering_inputs(session_id, steering)
            .await?;
        Ok(PendingInput::new(count, message_ids))
    }

    async fn close(&self, _session_id: SessionId) -> Result<()> {
        self.steering.close();
        Ok(())
    }

    async fn park(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
        client_tool_calls: Vec<ToolCall>,
        resume: TurnState,
    ) -> Result<()> {
        // No reason boundary remains, so accepted steering is appended with
        // its prompt hooks applied now; the park waits in memory for
        // `resume_steerable_turn`.
        self.steering.close();
        self.runtime
            .append_accepted_inputs(session_id, turn_id, self.steering.drain())
            .await?;
        self.runtime
            .park_turn(session_id, turn_id, client_tool_calls, resume);
        Ok(())
    }
}
