// Platform Management capability — retired.
//
// Decision: this handwritten capability was superseded by `platform`, whose
// catalog-backed `discover`/`query`/`execute` tools track the authoritative
// command inventory instead of a parallel, hand-maintained tool surface. Its
// implementation (14 tools, the system prompt, and the embedded docs mount,
// which moved to `platform`) is gone.
//
// The capability stays registered as an inert shell so agents, harnesses, and
// sessions that still reference `platform_management` keep resolving and
// running. `CapabilityStatus::Retired` contributes nothing at runtime and hides
// the capability from catalogs; management surfaces report it as removed so the
// reference can be dropped. See `knowledge/execution/capabilities.md`.

use everruns_core::capabilities::{Capability, CapabilityLocalization, CapabilityStatus};

pub const PLATFORM_MANAGEMENT_CAPABILITY_ID: &str = "platform_management";

pub struct PlatformManagementCapability;

impl Capability for PlatformManagementCapability {
    fn id(&self) -> &str {
        PLATFORM_MANAGEMENT_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Platform Management"
    }

    fn description(&self) -> &str {
        "Removed. Use the Platform capability, which exposes the same management surface through the authoritative command catalog."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Керування платформою",
            "Вилучено. Використовуйте можливість «Платформа», яка надає той самий обсяг керування через авторитетний каталог команд.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Retired
    }

    fn icon(&self) -> Option<&str> {
        Some("settings-2")
    }

    fn category(&self) -> Option<&str> {
        Some("Platform")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_capability_contributes_nothing() {
        let cap = PlatformManagementCapability;
        assert_eq!(cap.status(), CapabilityStatus::Retired);
        assert!(!cap.status().is_active());
        assert!(!cap.status().is_listed());
        assert!(cap.tools().is_empty());
        assert!(cap.tool_definitions().is_empty());
        assert!(cap.system_prompt_addition().is_none());
        assert!(cap.mounts().is_empty());
        assert!(cap.dependencies().is_empty());
    }
}
