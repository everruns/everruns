//! Startup and bootstrap: storage bring-up, service seeding, and organization
//! initialization.

// Organization initialization (built-in harnesses, reconciliation)
pub mod org_init;

// Service seeding (default agents, providers, models)
pub mod seed;

// Storage, durable event store, and turn backend bring-up
pub(crate) mod storage_init;
