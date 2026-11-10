//! The Engine on a worker (data plane v2 step 4): the execution-only hosting
//! profile, the storage ports a platform session runs on, the hydrator that
//! builds platform declarations and the session-files Environment.

mod backends;
mod deltas;
mod extensions;
mod files;
mod hosting;
mod hydrator;
mod journal;
mod lane;

pub use backends::{EventStamp, PlatformEventLog, PlatformSessionStore};
pub use files::{PlatformFiles, PlatformFilesBackend, PlatformWorkspace};
pub use hosting::{DEFINITION_ENVIRONMENT, LeaseSignals, PlatformDefinitions, PlatformHosting};
pub use hydrator::PlatformHydrator;
pub use lane::WorkerEngine;

#[cfg(test)]
mod tests;
