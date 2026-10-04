//! Hosted adapters for sandbox checkpoint contracts.

pub use everruns_contracts::sandbox_checkpoint::*;

/// Read access to durable tool-call state, installed on the extension seam.
/// Reconciliation checks whether the tool call that produced a checkpoint
/// committed before allowing its workspace revision to become authoritative.
#[derive(Clone)]
pub struct DurableToolResultStoreExt(
    pub std::sync::Arc<dyn everruns_core::durability::DurableToolResultStore>,
);
