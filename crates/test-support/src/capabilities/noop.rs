//! Noop Capability - for testing and demonstration purposes

use everruns_core::capabilities::{Capability, CapabilityLocalization, CapabilityStatus};

pub const NOOP_CAPABILITY_ID: &str = "noop";

/// Noop capability - for testing and demonstration purposes
pub struct NoopCapability;

impl Capability for NoopCapability {
    fn id(&self) -> &str {
        NOOP_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "No-Op"
    }

    fn description(&self) -> &str {
        "A no-operation capability for testing and demonstration purposes. Does not add any functionality."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Без дії",
            "Можливість без операцій для тестування та демонстрації. Не додає жодної функціональності.",
        )]
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
