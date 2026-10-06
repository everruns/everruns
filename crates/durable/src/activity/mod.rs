//! Activity abstractions
//!
//! Activities are units of work that are executed by workers. They:
//! - May fail and be retried according to the retry policy
//! - Can send heartbeats to indicate liveness
//! - Support cancellation via tokens
//!
//! [`ActivityError`] is part of the event log and always available. The
//! `Activity` trait and `ActivityContext` belong to the experimental
//! `workflows` feature.

#[cfg(feature = "workflows")]
mod context;
#[cfg(feature = "workflows")]
mod definition;
mod error;

#[cfg(feature = "workflows")]
pub use context::ActivityContext;
#[cfg(feature = "workflows")]
pub use definition::Activity;
pub use error::ActivityError;
