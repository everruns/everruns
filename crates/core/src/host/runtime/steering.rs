//! Steering: user input that joins a running turn at its next reason
//! boundary.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use everruns_contracts::error::Result;
use everruns_contracts::typed_id::{MessageId, SessionId};

use super::{AcceptedTurnInput, InProcessRuntime};
use crate::event_emitter::EventEmitter;
use crate::events::{EventContext, EventRequest, InputMessageData};

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
}
