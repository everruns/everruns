#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
//! Wire types for the [AG-UI](https://docs.ag-ui.com) 1.0 protocol, part of
//! the [Everruns](https://everruns.com) ecosystem.
//!
//! AG-UI is the event protocol between an agent and the application that
//! renders it: the application posts a [`RunAgentInput`](crate::ag_ui::RunAgentInput), the agent answers
//! with a stream of [`Event`]s (usually over SSE).
//!
//! ```
//! use everruns_core::ag_ui::{Event, RunFinishedEvent, RunFinishedOutcome, RunStartedEvent};
//!
//! let started = Event::RunStarted(
//!     RunStartedEvent::new("thread-1", "run-1").with_protocol_version(),
//! );
//! let finished = Event::RunFinished(
//!     RunFinishedEvent::new("thread-1", "run-1").with_outcome(RunFinishedOutcome::Cancelled),
//! );
//!
//! assert_eq!(
//!     serde_json::to_value(&started).unwrap(),
//!     serde_json::json!({
//!         "type": "RUN_STARTED",
//!         "threadId": "thread-1",
//!         "runId": "run-1",
//!         "protocolVersion": "1.0",
//!     }),
//! );
//! assert_eq!(
//!     serde_json::to_value(&finished).unwrap()["outcome"],
//!     serde_json::json!({ "type": "cancelled" }),
//! );
//! ```
//!
//! # Modules
//!
//! - The wire types, at this module’s root (feature `ag-ui`).
//! - [`consumer`](crate::ag_ui::consumer): the consumer side of the protocol. It decodes a
//!   producer's events, enforces the 1.0 sequencing rules and assembles a
//!   [`consumer::RunResult`](crate::ag_ui::consumer::RunResult); [`ResumeBuilder`](crate::ag_ui::ResumeBuilder) answers interrupts.
//! - `client` (feature `ag-ui-client`): an HTTP client that runs an AG-UI agent
//!   over SSE and feeds the consumer.
//! - `projection` (feature `ag-ui-projection`): Everruns runtime events as an AG-UI run.
//!
//! # Contract
//!
//! - The types follow the pinned upstream schema in `spec/1.0/schema.json`;
//!   the upstream fixture corpus in `spec/1.0/fixtures` is this crate's test
//!   suite.
//! - **Absent means absent.** Optional fields serialize as omitted, never as
//!   `null`, so everything this crate emits validates against the schema.
//! - **Tolerant on input.** Deserialization ignores unknown fields and reads a
//!   historical whole-field `null` (`forwardedProps`, `parentMessageId`, ...)
//!   as absent, which is how 1.0 consumers treat 0.x producers.
//! - Fields the schema leaves open (`result`, `payload`, `state`, tool
//!   `parameters`) are [`serde_json::Value`]. The schema forbids `null` for
//!   them, so leave them `None` rather than `Some(Value::Null)`.

// Decision: hand-written serde types rather than code generated from the
// schema. The schema leans on `allOf` + `unevaluatedProperties` and `const`
// discriminators that Rust generators turn into unidiomatic types, and the
// upstream fixtures plus schema validation of everything we serialize (see
// `tests/spec.rs`) catch drift as well as generation would.

mod capabilities;
#[cfg(feature = "ag-ui-client")]
pub mod client;
pub mod consumer;
mod event;
mod input;
mod message;
mod patch;
#[cfg(feature = "ag-ui-projection")]
pub mod projection;
mod resume;

pub use capabilities::*;
pub use event::*;
pub use input::*;
pub use message::*;
pub use patch::*;
pub use resume::*;

/// The AG-UI protocol version these types implement.
pub const PROTOCOL_VERSION: &str = "1.0";

/// Open-by-key metadata carried by events, messages and interrupts.
///
/// Per key, last write wins and merges never recurse. The `ag-ui` key is
/// reserved for the protocol.
pub type Metadata = serde_json::Map<String, serde_json::Value>;

/// The upstream JSON Schema these types implement, verbatim.
pub const SCHEMA_JSON: &str = include_str!("spec/1.0/schema.json");
