//! The consumer side of AG-UI 1.0: turning a producer's event stream into
//! a checked, assembled [`RunResult`].
//!
//! Three steps, usable together through [`RunConsumer`] or one by one:
//!
//! 1. [`decode_event`] applies the 1.0 processing model to one wire value:
//!    unknown event types are dropped, unknown union members stripped,
//!    undescribed properties ignored, malformed known values rejected.
//! 2. [`Verifier`] expands the `*_CHUNK` shorthand and enforces the
//!    sequencing rules: `RUN_STARTED` first, nothing but a new run after a
//!    terminal event, every message, tool call, reasoning span, step and
//!    subagent opened before it continues and closed before the run
//!    finishes, and continuations agreeing with their opener's attribution.
//! 3. [`RunConsumer`] folds what survives into a [`RunResult`]: messages,
//!    tool calls, outcome, interrupts and usage.
//!
//! The rules follow the reference TypeScript client, and the upstream
//! client conformance corpus (`spec/1.0/conformance` in this crate's
//! repository) is the test suite. The resume side of interrupts lives in
//! [`crate::ag_ui::ResumeBuilder`].
//!
//! ```
//! use everruns_core::ag_ui::consumer::{RunConsumer, RunOutcome};
//! use serde_json::json;
//!
//! let mut consumer = RunConsumer::new();
//! consumer.push_value(json!({ "type": "RUN_STARTED", "threadId": "t", "runId": "r" })).unwrap();
//! consumer
//!     .push_value(json!({
//!         "type": "RUN_FINISHED", "threadId": "t", "runId": "r",
//!         "outcome": { "type": "interrupt", "interrupts": [{ "id": "i1", "reason": "approval" }] },
//!     }))
//!     .unwrap();
//! let result = consumer.finish().unwrap();
//! assert_eq!(result.outcome, RunOutcome::Interrupted);
//! assert_eq!(result.interrupts[0].id, "i1");
//!
//! // Anything after RUN_FINISHED other than a new run is a violation.
//! let mut consumer = RunConsumer::new();
//! consumer.push_value(json!({ "type": "RUN_STARTED", "threadId": "t", "runId": "r" })).unwrap();
//! consumer.push_value(json!({ "type": "RUN_FINISHED", "threadId": "t", "runId": "r" })).unwrap();
//! let late = json!({ "type": "TEXT_MESSAGE_START", "messageId": "m" });
//! assert!(consumer.push_value(late).is_err());
//! ```

mod decode;
mod run;
mod verify;

pub use decode::{Decoded, decode_event};
pub use run::{
    AssembledMessage, AssembledToolCall, RunConsumer, RunOutcome, RunResult, merge_usage,
};
pub use verify::Verifier;

/// A producer broke the protocol. The stream should be abandoned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtocolError {
    message: String,
}

impl ProtocolError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The violation, in words.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AG-UI protocol violation: {}", self.message)
    }
}

impl std::error::Error for ProtocolError {}
