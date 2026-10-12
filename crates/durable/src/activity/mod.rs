//! Activity errors
//!
//! Activities are units of work that workers run. They may fail and be
//! retried according to the retry policy. [`ActivityError`] is how a failure is
//! recorded in the event log and the task queue.

mod error;

pub use error::ActivityError;
