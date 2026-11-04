//! An opt-in analytical copy of canonical events, never an execution journal.
//!
//! Objects land before their catalog transaction commits. Per-session locks
//! serialize export, compaction, and deletion; queries see only published files
//! belonging to live sessions in their org. PostgreSQL events are never removed.

mod backfill;
mod catalog;
mod companion;
mod index_cache;
mod model;
mod monitor;
mod query;
mod schema;
mod segment;
mod shadow;
mod store;

pub use backfill::BackfillStatus;
pub use companion::{CycleReport, Progress};
pub use model::{
    CacheStats, EventQuery, EventRecord, EventSummary, Limits, QueryPage, ReadEngine, ReadProfile,
    Scope,
};
pub use monitor::Monitor;
pub use schema::migrate_catalog;
pub use shadow::{ShadowOutcome, ShadowReport};
pub use store::TraceStore;

/// Shared idempotent DDL for the companion and later platform adoption.
pub const CATALOG_MIGRATION: &str =
    include_str!("../../server/migrations/20261102213413_trace_store_segments.sql");

pub const PARALLEL_MIGRATION: &str =
    include_str!("../../server/migrations/20261103015837_trace_store_parallel.sql");

#[cfg(test)]
mod tests;
