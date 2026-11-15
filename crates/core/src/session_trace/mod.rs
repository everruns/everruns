//! Pure event-to-turn/step projection shared by Framework and Server.
pub mod from_events;
pub mod projection;
pub use from_events::trace_event;
pub use projection::{TRACE_EVENT_TYPES, TraceEvent, TraceWorkingSet};
