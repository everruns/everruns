//! The Engine on a worker (data plane v2 step 4): the execution-only hosting
//! profile, the storage ports a platform session runs on, the hydrator that
//! builds platform declarations and the session-files Environment.

mod backends;
mod files;
mod hosting;
mod hydrator;
mod lane;

pub use backends::{PlatformEventLog, PlatformSessionStore};
pub use files::{PlatformFiles, PlatformFilesBackend};
pub use hosting::{DEFINITION_ENVIRONMENT, LeaseSignals, PlatformDefinitions, PlatformHosting};
pub use hydrator::PlatformHydrator;
pub use lane::WorkerEngine;
