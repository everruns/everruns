//! The backend conformance suite: the same scenarios on every turn backend.
//!
//! Each scenario runs once on the in-process backend (the default) and once on
//! the durable memory backend (`durable::Backend::memory`), and the two must
//! observe the same outcome: the turns' answers and shape (stop reason,
//! iterations, tool calls), the session's persisted event types in order, and
//! whatever the scenario notes on the way (send dispositions, parked calls,
//! AG-UI event types). `TurnBackend` stays experimental until this suite
//! passes on both (see `knowledge/framework/execution-backends.md`).
//!
//! Scenarios:
//! - `turns`: a single turn, a tool loop, steering into the running turn
//!   versus starting the next one, and cancel then the next turn.
//! - `parked`: a turn parks on a client-side tool call and its result resumes
//!   it, through a session's AG-UI runs and directly on the backend seam.
//! - `interrupted`: a turn whose act is cut off when its session goes away,
//!   then `resume_interrupted_turn` on the reopened session finishes it.
//!
//! A backend difference the suite tolerates is spelled out where the scenario
//! encodes it; none is compared loosely.
//!
//! Run with: `cargo test -p everruns --features durable,ag-ui --test backend_conformance`

mod interrupted;
mod parked;
mod support;
mod turns;
