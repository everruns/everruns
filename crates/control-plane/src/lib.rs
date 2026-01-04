// Everruns Control Plane Library
// Decision: Shared library for binaries (API server, CLI tools)

// Event service is exposed at library level for use by storage layer
// (other services remain binary-only as they depend on API types)
mod services {
    pub mod event;
    pub use event::EventService;
}

pub use services::EventService;
pub mod storage;
