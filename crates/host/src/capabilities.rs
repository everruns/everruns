//! Capability composition for embedded hosts.
//!
//! `everruns-core` owns only effect-neutral capability contracts and built-ins.
//! This module is the opt-in host composition boundary for environment-backed
//! implementations.

use std::sync::Arc;

use everruns_core::{CapabilityRegistry, EgressService};

/// Return the runtime-safe core built-ins plus integrations enabled as host
/// Cargo features.
pub fn runtime_capability_registry() -> CapabilityRegistry {
    compose_runtime_capability_registry(CapabilityRegistry::runtime_builtins())
}

/// Add host integrations selected by Cargo features to an existing registry.
///
/// This lets embedders preserve a broader core preset (for example, the local
/// profile's schedule capability) while applying the same feature-driven
/// environment composition as [`runtime_capability_registry`]. Existing
/// registrations retain the registry's normal duplicate handling.
pub fn compose_runtime_capability_registry(mut registry: CapabilityRegistry) -> CapabilityRegistry {
    register_selected_integrations(&mut registry);
    registry
}

/// Return the egress service matching the selected host integrations.
///
/// Network-capable opt-in features retain the Framework's direct, policy-aware
/// transport. A host with no network-capable feature remains offline and does
/// not link the concrete HTTP crate.
pub fn runtime_egress_service() -> Arc<dyn EgressService> {
    #[cfg(any(
        feature = "bashkit",
        feature = "web-fetch",
        feature = "lua",
        feature = "mcp"
    ))]
    {
        Arc::new(everruns_http::DirectEgressService::for_runtime_traffic_from_env())
    }
    #[cfg(not(any(
        feature = "bashkit",
        feature = "web-fetch",
        feature = "lua",
        feature = "mcp"
    )))]
    {
        Arc::new(everruns_core::DisabledEgressService)
    }
}

fn register_selected_integrations(_registry: &mut CapabilityRegistry) {
    #[cfg(feature = "filesystem")]
    _registry.register(everruns_integrations_filesystem::FileSystemCapability);
    #[cfg(feature = "bashkit")]
    _registry.register(everruns_integrations_bashkit::BashkitShellCapability);
    #[cfg(feature = "web-fetch")]
    _registry.register(everruns_integrations_web_fetch::WebFetchCapability::from_env());
    #[cfg(feature = "lua")]
    if everruns_core::InternalFeatureFlags::from_env().lua {
        _registry.register(everruns_integrations_lua::LuaCapability);
        _registry.register(everruns_integrations_lua::LuaCodeModeCapability);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_registry_matches_selected_host_features() {
        let registry = runtime_capability_registry();
        assert_eq!(
            registry.has("session_file_system"),
            cfg!(feature = "filesystem")
        );
        assert_eq!(registry.has("bashkit_shell"), cfg!(feature = "bashkit"));
        assert_eq!(registry.has("web_fetch"), cfg!(feature = "web-fetch"));

        if cfg!(feature = "bashkit") {
            assert_eq!(registry.canonical_id("virtual_bash"), Some("bashkit_shell"));
        }
    }

    #[test]
    fn composition_preserves_caller_selected_core_capabilities() {
        let registry = compose_runtime_capability_registry(CapabilityRegistry::with_builtins());
        // A core-owned capability the caller brought in survives composition.
        // `session_schedule` used to stand in here; it is hosted and lives in
        // everruns-platform now (EVE-885), so it is no longer in this preset.
        assert!(registry.has("session_storage"));
        assert_eq!(
            registry.has("session_file_system"),
            cfg!(feature = "filesystem")
        );
    }

    #[test]
    fn runtime_egress_matches_network_feature_selection() {
        let expected = cfg!(any(
            feature = "bashkit",
            feature = "web-fetch",
            feature = "lua",
            feature = "mcp"
        ));
        assert_eq!(
            runtime_egress_service().name() == "DirectEgressService",
            expected
        );
    }
}
