//! Portable memory consolidation. Hosts own durable storage and wakeups.
use crate::dreaming::*;
#[cfg(any(feature = "host", feature = "execution-host", test))]
pub(crate) mod evidence;
pub mod runner;
pub mod store;
pub use runner::{DreamActivity, DreamEvidence, DreamJob, DreamNote, DreamOutput, DreamingHost};
pub use store::{InMemoryDreamingStore, UnsupportedDreamingStore};
#[cfg(test)]
mod tests;

mod queue;
