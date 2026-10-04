//! Host-owned capabilities; integration selection belongs to the facade.

#[cfg(feature = "host-shell")]
pub mod shell;

/// Create the host-service registry without attaching drivers or integrations.
pub fn runtime_capability_registry() -> crate::CapabilityRegistry {
    compose_runtime_capability_registry(crate::CapabilityRegistry::new())
}

/// Register the services supplied by an in-process host on an explicit registry.
pub fn compose_runtime_capability_registry(
    mut registry: crate::CapabilityRegistry,
) -> crate::CapabilityRegistry {
    if registry.get(crate::host::SESSION_CAPABILITY_ID).is_none() {
        registry.register(crate::host::SessionCapability);
    }
    if registry
        .get(crate::host::SESSION_STORAGE_CAPABILITY_ID)
        .is_none()
    {
        registry.register(crate::host::SessionStorageCapability);
    }
    registry
}

/// Hosts stay offline until their composition supplies an outbound transport.
pub fn runtime_egress_service() -> std::sync::Arc<dyn crate::EgressService> {
    std::sync::Arc::new(crate::DisabledEgressService)
}
