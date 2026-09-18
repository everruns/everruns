//! Open, value-first capability configuration for Framework agents.
//!
//! These are the neutral `everruns-capability` contract types re-exported at
//! their stable Framework paths (EVE-873). Downstream capability authors can
//! keep depending only on `everruns`, or depend on the dependency-light
//! `everruns-capability` crate directly — both expose the same
//! [`CapabilityRef`]/[`CapabilitySpec`]/[`IntoCapability`] contract, and the
//! same reference representation round-trips through hosted product
//! attachments and worker resolution.
//!
//! # Example
//!
//! ```
//! use everruns::{CapabilityRef, CapabilitySpec, IntoCapability};
//! use serde_json::json;
//!
//! struct VendorSearch {
//!     index: String,
//! }
//!
//! impl IntoCapability for VendorSearch {
//!     fn into_capability(self) -> CapabilitySpec {
//!         CapabilityRef::new("vendor.search")
//!             .config(json!({ "index": self.index }))
//!             .into()
//!     }
//! }
//! ```

pub use everruns_capability::{CapabilityRef, CapabilitySpec, IntoCapability};

use crate::agent::BuildError;

pub(crate) fn framework_capability_registry(
    hosted_base: bool,
) -> everruns_core::CapabilityRegistry {
    #[cfg(not(feature = "builtins"))]
    let _ = hosted_base;
    let registry = everruns_core::CapabilityRegistry::new();
    #[cfg(feature = "builtins")]
    let registry = {
        let mut registry = registry;
        if hosted_base {
            everruns_builtins::register_portable_capabilities(&mut registry)
                .expect("portable built-in catalog must have unique capability IDs");
        } else {
            everruns_builtins::register_runtime_capabilities(&mut registry)
                .expect("portable runtime catalog must have unique capability IDs");
        }
        registry
    };
    everruns_host::compose_runtime_capability_registry(registry)
}

pub(crate) fn validate_registered_capability_config(
    registry: &everruns_core::CapabilityRegistry,
    id: &str,
    config: &serde_json::Value,
) -> Result<(), BuildError> {
    if let Some(capability) = registry.get(id) {
        capability
            .validate_config(config)
            .map_err(|reason| BuildError::InvalidCapability {
                id: id.to_string(),
                reason,
            })?;
    }

    #[cfg(feature = "builtins")]
    if id == everruns_builtins::AUTO_TOOL_SEARCH_CAPABILITY_ID {
        crate::agent::validate_tool_search_config(config).map_err(|reason| {
            BuildError::InvalidCapability {
                id: id.to_string(),
                reason,
            }
        })?;
    }

    if everruns_core::is_declarative_capability(id) || everruns_capability::is_plugin_capability(id)
    {
        let mut definition =
            serde_json::from_value::<everruns_core::DeclarativeCapabilityDefinition>(
                config.clone(),
            )
            .map_err(|error| BuildError::InvalidCapability {
                id: id.to_string(),
                reason: format!("invalid declarative capability config: {error}"),
            })?;
        // Plugin identities have already been validated by their compiler and
        // intentionally allow names outside the narrower declarative contract.
        if everruns_capability::is_plugin_capability(id) {
            definition.name = "plugin".to_string();
        }
        everruns_core::validate_declarative_capability_definition(&definition).map_err(
            |reason| BuildError::InvalidCapability {
                id: id.to_string(),
                reason,
            },
        )?;
    }
    Ok(())
}
