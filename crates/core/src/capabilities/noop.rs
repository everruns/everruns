//! Noop Capability - for testing and demonstration purposes

use super::{Capability, CapabilityId, CapabilityStatus};

/// Noop capability - for testing and demonstration purposes
pub struct NoopCapability;

impl Capability for NoopCapability {
    fn id(&self) -> &str {
        CapabilityId::NOOP
    }

    fn name(&self) -> &str {
        "No-Op"
    }

    fn description(&self) -> &str {
        "A no-operation capability for testing and demonstration purposes. Does not add any functionality."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("circle-off")
    }

    fn category(&self) -> Option<&str> {
        Some("Testing")
    }
}
