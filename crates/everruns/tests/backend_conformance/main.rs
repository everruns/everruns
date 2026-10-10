//! The backend conformance suite: the scenarios every turn backend must pass.
//!
//! Engine scenarios run on an engine's sessions, which run as actors
//! (`ActorRunner`: in process, under the session's lease). The entry-point
//! scenarios start the same turns directly on `InProcessBackend` and on
//! `ActorRunner` and require the same outcome: the turns' answers and shape
//! (stop reason, iterations, tool calls), the session's persisted event types
//! in order, and whatever the scenario notes on the way (send dispositions,
//! parked calls, AG-UI event types). A third-party `TurnBackend`, and each
//! store the actor runner gains, is held to the same bar (see
//! `knowledge/framework/execution-backends.md`).
//!
//! Scenarios:
//! - `turns`: a single turn, a tool loop, steering into the running turn
//!   versus starting the next one, and cancel then the next turn.
//! - `parked`: a turn parks on a client-side tool call and its result resumes
//!   it, through a session's AG-UI runs and directly on the turn entry point,
//!   with input handed to the backend and with input the caller stored.
//! - `interrupted`: a turn whose act is cut off when its session goes away,
//!   then `resume_interrupted_turn` on the reopened session finishes it.
//!
//! Run with: `cargo test -p everruns --features ag-ui --test backend_conformance`

mod interrupted;
mod parked;
mod support;
mod turns;
