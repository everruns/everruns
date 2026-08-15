//! Capability composition for embedded hosts.
//!
//! `everruns-core` owns only neutral capability contracts and registry algorithms.
//! This module is the opt-in host composition boundary for environment-backed
//! implementations.

use std::sync::Arc;

use everruns_core::{CapabilityRegistry, EgressService};

/// Return the runtime-safe portable bundle and integrations enabled as host Cargo features.
pub fn runtime_capability_registry() -> CapabilityRegistry {
    let registry = CapabilityRegistry::new();
    #[cfg(feature = "builtins")]
    let registry = {
        let mut registry = registry;
        everruns_builtins::register_runtime_capabilities(&mut registry)
            .expect("portable runtime catalog must have unique capability IDs");
        registry
    };
    compose_runtime_capability_registry(registry)
}

/// Add host integrations selected by Cargo features to an existing registry.
///
/// This lets embedders preserve a caller-owned registry while applying the same feature-driven
/// environment composition as [`runtime_capability_registry`]. Existing
/// registrations retain the registry's normal duplicate handling.
pub fn compose_runtime_capability_registry(mut registry: CapabilityRegistry) -> CapabilityRegistry {
    register_session_service_capabilities(&mut registry);
    register_selected_integrations(&mut registry);
    registry
}

/// Register the session-service capabilities an in-process host can serve.
///
/// `session` and `session_storage` moved to the product crate with the rest of
/// the service-backed families (EVE-886), but the default in-process runtime
/// supplies both services, so the Framework keeps advertising them. The SQL and
/// sandbox capabilities need backends this host does not provide and stay with
/// product composition.
fn register_session_service_capabilities(registry: &mut CapabilityRegistry) {
    if registry
        .get(everruns_platform::capabilities::SESSION_CAPABILITY_ID)
        .is_none()
    {
        registry.register(everruns_platform::capabilities::SessionCapability);
    }
    if registry
        .get(everruns_platform::capabilities::SESSION_STORAGE_CAPABILITY_ID)
        .is_none()
    {
        registry.register(everruns_platform::capabilities::SessionStorageCapability);
    }
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
        feature = "duckduckgo",
        feature = "lua",
        feature = "mcp"
    ))]
    {
        Arc::new(everruns_http::DirectEgressService::for_runtime_traffic_from_env())
    }
    #[cfg(not(any(
        feature = "bashkit",
        feature = "web-fetch",
        feature = "duckduckgo",
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
    #[cfg(feature = "duckduckgo")]
    _registry.register(everruns_integrations_duckduckgo::DuckDuckGoCapability);
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
        assert_eq!(registry.has("human_intent"), cfg!(feature = "builtins"));
        assert_eq!(registry.has("infinity_context"), cfg!(feature = "builtins"));
        assert_eq!(registry.has("skills"), cfg!(feature = "builtins"));
        assert_eq!(registry.has("current_time"), cfg!(feature = "builtins"));
        assert_eq!(registry.has("compaction"), cfg!(feature = "builtins"));
        assert!(
            !registry.has("usage_limit_auto_continue"),
            "the embedded runtime preset has no schedule-backed auto-continue service"
        );
        assert_eq!(
            registry.has("session_file_system"),
            cfg!(feature = "filesystem")
        );
        assert_eq!(registry.has("bashkit_shell"), cfg!(feature = "bashkit"));
        assert_eq!(registry.has("web_fetch"), cfg!(feature = "web-fetch"));
        assert_eq!(registry.has("duckduckgo"), cfg!(feature = "duckduckgo"));
        assert!(!registry.has("openui"));
        assert!(!registry.has("a2ui"));
        assert!(!registry.has("openrouter_server_tools"));

        if cfg!(feature = "bashkit") {
            assert_eq!(registry.canonical_id("virtual_bash"), Some("bashkit_shell"));
        }
    }

    #[test]
    fn composition_preserves_caller_selected_core_capabilities() {
        let mut selected = CapabilityRegistry::new();
        #[cfg(feature = "builtins")]
        everruns_builtins::register_runtime_capabilities(&mut selected).unwrap();
        let registry = compose_runtime_capability_registry(selected);
        // A caller-selected capability survives environment composition.
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
            feature = "duckduckgo",
            feature = "lua",
            feature = "mcp"
        ));
        assert_eq!(
            runtime_egress_service().name() == "DirectEgressService",
            expected
        );
    }
}
