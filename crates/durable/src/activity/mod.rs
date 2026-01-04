//! Activity abstractions
//!
//! Activities are units of work that are executed by workers. They:
//! - May fail and be retried according to the retry policy
//! - Can send heartbeats to indicate liveness
//! - Support cancellation via tokens

mod context;
mod definition;

pub use context::ActivityContext;
pub use definition::{Activity, ActivityError};
