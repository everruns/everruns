//! Capabilities: the ones this host composes, and the ones it implements.
//!
//! `everruns-core` owns only neutral capability contracts and registry
//! algorithms. This module is the opt-in host composition boundary for
//! environment-backed implementations, and the home for the capability
//! implementations `everruns-host` owns itself.
//!
//! A capability lands here when it is an *embedder* capability: something a CLI
//! host, a CI runner, or an operator's own box opts into, rather than something
//! the hosted product offers every tenant. Those go in an integration crate and
//! are named by `everruns-integrations-catalog`.
//!
//! One pair lives elsewhere by design. `session` and `session_storage` are the
//! capability face of the session-service seam and sit under
//! [`session_services`](crate::session_services) next to `session_mutator`,
//! which `session` depends on. A capability that fronts another seam belongs
//! with that seam; everything else belongs here.

use std::sync::Arc;

use everruns_core::{CapabilityRegistry, EgressService};

#[cfg(feature = "host-shell")]
pub mod shell;

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
    if registry.get(crate::SESSION_CAPABILITY_ID).is_none() {
        registry.register(crate::SessionCapability);
    }
    if registry.get(crate::SESSION_STORAGE_CAPABILITY_ID).is_none() {
        registry.register(crate::SessionStorageCapability);
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
        Arc::new(crate::DirectEgressService::for_runtime_traffic_from_env())
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
    // Both contribute a tool named `bash`, so an embedder selects one. Nothing
    // stops both features being on at once; the capability an agent enables is
    // what decides which shell it gets.
    #[cfg(feature = "host-shell")]
    _registry.register(shell::HostShellCapability);
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
        assert_eq!(registry.has("host_shell"), cfg!(feature = "host-shell"));
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
        let selected = CapabilityRegistry::new();
        #[cfg(feature = "builtins")]
        let selected = {
            let mut selected = selected;
            everruns_builtins::register_runtime_capabilities(&mut selected).unwrap();
            selected
        };
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
