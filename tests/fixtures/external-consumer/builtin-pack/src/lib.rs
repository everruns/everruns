//! Downstream composition proof for the portable built-in policy bundle.

use everruns_builtins::register_portable_capabilities;
use everruns_core::CapabilityRegistry;

/// Compose the portable policy bundle exactly as a custom host would.
pub fn registry() -> CapabilityRegistry {
    let mut registry = CapabilityRegistry::new();
    register_portable_capabilities(&mut registry)
        .expect("a fresh registry cannot collide with the portable catalog");
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_host_registers_the_policy_bundle_explicitly() {
        let registry = registry();
        assert!(registry.has("compaction"));
        assert!(registry.has("auto_tool_search"));
        assert!(registry.has("tool_call_repair"));
        assert!(!registry.has("web_fetch"));
        assert!(!registry.has("platform"));
    }
}
