#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
#![deny(missing_docs)]

//! The catalog of integration crates composed into the hosted Everruns product.
//!
//! Every integration crate publishes its capabilities and connectors as plain
//! `const` slices. This crate names each one exactly once in [`CATALOG`], so
//! the composition is a compile-checked list rather than something the linker
//! decides.
//!
//! This crate is part of the [Everruns](https://everruns.com) ecosystem.
//!
//! Embedders that want a different set start from [`CATALOG`], filter or
//! extend it, and register the result — or skip it entirely and register
//! capabilities directly on a registry they own.
//!
//! # Example
//!
//! ```
//! use everruns_core::DeploymentGrade;
//! use everruns_integrations_catalog::{CATALOG, capability_is_enabled};
//!
//! // Every integration is named exactly once and contributes something.
//! assert!(CATALOG.iter().all(|entry| {
//!     !entry.capabilities.is_empty() || !entry.connectors.is_empty()
//! }));
//!
//! // Experimental integrations stay out of a production registry.
//! for entry in CATALOG {
//!     for plugin in entry.capabilities {
//!         if plugin.experimental_only {
//!             assert!(!capability_is_enabled(plugin, DeploymentGrade::Prod));
//!         }
//!     }
//! }
//! ```

// Registration used to happen through `inventory::submit!`, with `extern crate`
// lines in `everruns-server` and `everruns-worker` forcing the integration
// crates to link so their submissions survived. That made a registry's contents
// a linker side effect: a forgotten line dropped an integration with no compile
// error, the list was maintained in four places, and the same registry differed
// between binaries depending on what was linked. Naming the catalog costs one
// entry per integration, lets the compiler check it, and makes linkage a
// consequence of a real reference.

use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_core::{DeploymentGrade, ExecutionFeatureDecisions};
use everruns_platform::connector::{ConnectorPlugin, ConnectorRegistry};

/// One integration crate's contribution to the hosted product.
pub struct CatalogEntry {
    /// The contributing crate, for diagnostics and for embedders filtering the catalog.
    pub crate_name: &'static str,
    /// Capabilities the crate contributes.
    pub capabilities: &'static [IntegrationPlugin],
    /// Connectors the crate contributes.
    pub connectors: &'static [ConnectorPlugin],
}

/// Every integration composed into the hosted product, in registration order.
///
/// Registration order decides which contributor wins a canonical-id collision:
/// [`CapabilityRegistry::register_plugins`] and [`ConnectorRegistry`] both let
/// a later entry replace an earlier one.
pub const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        crate_name: "everruns-ard",
        capabilities: everruns_ard::CAPABILITY_PLUGINS,
        connectors: everruns_ard::CONNECTOR_PLUGINS,
    },
    CatalogEntry {
        crate_name: "everruns-integrations-brave-search",
        capabilities: everruns_integrations_brave_search::CAPABILITY_PLUGINS,
        connectors: everruns_integrations_brave_search::CONNECTOR_PLUGINS,
    },
    CatalogEntry {
        crate_name: "everruns-integrations-browserless",
        capabilities: everruns_integrations_browserless::CAPABILITY_PLUGINS,
        connectors: everruns_integrations_browserless::CONNECTOR_PLUGINS,
    },
    CatalogEntry {
        crate_name: "everruns-integrations-cursor",
        capabilities: everruns_integrations_cursor::CAPABILITY_PLUGINS,
        connectors: everruns_integrations_cursor::CONNECTOR_PLUGINS,
    },
    CatalogEntry {
        crate_name: "everruns-integrations-daytona",
        capabilities: everruns_integrations_daytona::CAPABILITY_PLUGINS,
        connectors: everruns_integrations_daytona::CONNECTOR_PLUGINS,
    },
    CatalogEntry {
        crate_name: "everruns-integrations-deno",
        capabilities: everruns_integrations_deno::CAPABILITY_PLUGINS,
        connectors: everruns_integrations_deno::CONNECTOR_PLUGINS,
    },
    CatalogEntry {
        crate_name: "everruns-integrations-docker",
        capabilities: everruns_integrations_docker::CAPABILITY_PLUGINS,
        connectors: &[],
    },
    CatalogEntry {
        crate_name: "everruns-integrations-duckduckgo",
        capabilities: everruns_integrations_duckduckgo::CAPABILITY_PLUGINS,
        connectors: &[],
    },
    CatalogEntry {
        crate_name: "everruns-integrations-e2b",
        capabilities: everruns_integrations_e2b::CAPABILITY_PLUGINS,
        connectors: everruns_integrations_e2b::CONNECTOR_PLUGINS,
    },
    CatalogEntry {
        crate_name: "everruns-integrations-github",
        capabilities: everruns_integrations_github::CAPABILITY_PLUGINS,
        connectors: &[],
    },
    CatalogEntry {
        crate_name: "everruns-integrations-openai-image",
        capabilities: everruns_integrations_openai_image::CAPABILITY_PLUGINS,
        connectors: &[],
    },
    CatalogEntry {
        crate_name: "everruns-integrations-parallel",
        capabilities: everruns_integrations_parallel::CAPABILITY_PLUGINS,
        connectors: everruns_integrations_parallel::CONNECTOR_PLUGINS,
    },
    CatalogEntry {
        crate_name: "everruns-integrations-sprites",
        capabilities: everruns_integrations_sprites::CAPABILITY_PLUGINS,
        connectors: everruns_integrations_sprites::CONNECTOR_PLUGINS,
    },
    CatalogEntry {
        crate_name: "everruns-integrations-typesafe",
        capabilities: everruns_integrations_typesafe::CAPABILITY_PLUGINS,
        connectors: everruns_integrations_typesafe::CONNECTOR_PLUGINS,
    },
];

/// Every capability plugin in [`CATALOG`], ungated.
pub fn capability_plugins() -> impl Iterator<Item = &'static IntegrationPlugin> {
    CATALOG.iter().flat_map(|entry| entry.capabilities.iter())
}

/// Every connector plugin in [`CATALOG`], ungated.
pub fn connector_plugins() -> impl Iterator<Item = &'static ConnectorPlugin> {
    CATALOG.iter().flat_map(|entry| entry.connectors.iter())
}

/// Whether a capability plugin is registered for `grade` under the current environment.
///
/// `experimental_only` is a grade gate; `feature_flag` resolves through
/// [`ExecutionFeatureDecisions`], which is fail-closed — an unset flag is off
/// at every grade.
pub fn capability_is_enabled(plugin: &IntegrationPlugin, grade: DeploymentGrade) -> bool {
    let decisions = ExecutionFeatureDecisions::from_env(grade);
    (!plugin.experimental_only || grade.experimental_features_enabled())
        && plugin
            .feature_flag
            .is_none_or(|flag| decisions.is_enabled(flag))
}

/// Register the catalog's capabilities accepted by `grade` onto `registry`.
pub fn register_capabilities(registry: &mut CapabilityRegistry, grade: DeploymentGrade) {
    let decisions = ExecutionFeatureDecisions::from_env(grade);
    registry.register_plugins(capability_plugins(), |plugin| {
        (!plugin.experimental_only || grade.experimental_features_enabled())
            && plugin
                .feature_flag
                .is_none_or(|flag| decisions.is_enabled(flag))
    });
}

/// The default OSS capability registry: portable builtins, then this catalog's
/// integrations, then the hosted product capabilities.
///
/// Decision: integrations register between builtins and hosted capabilities so
/// a hosted capability still wins a canonical-id collision, which is the order
/// the inventory-based composition had.
pub fn oss_capability_registry_for_grade(grade: DeploymentGrade) -> CapabilityRegistry {
    let mut registry = everruns_platform::capabilities::portable_capability_registry();
    register_capabilities(&mut registry, grade);
    everruns_platform::capabilities::register_hosted_capabilities(&mut registry, grade);
    registry
}

/// [`oss_capability_registry_for_grade`] using the environment grade.
pub fn oss_capability_registry() -> CapabilityRegistry {
    oss_capability_registry_for_grade(DeploymentGrade::from_env())
}

/// Register the catalog's connectors accepted by `grade` onto `registry`.
pub fn register_connectors(registry: &mut ConnectorRegistry, grade: DeploymentGrade) {
    for plugin in connector_plugins() {
        if plugin.experimental_only && !grade.experimental_features_enabled() {
            continue;
        }
        registry.register_boxed((plugin.factory)());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_contributes_something() {
        for entry in CATALOG {
            assert!(
                !entry.capabilities.is_empty() || !entry.connectors.is_empty(),
                "{} is in the catalog but contributes nothing",
                entry.crate_name
            );
        }
    }

    #[test]
    fn crate_names_are_unique() {
        let mut names: Vec<_> = CATALOG.iter().map(|entry| entry.crate_name).collect();
        names.sort_unstable();
        let total = names.len();
        names.dedup();
        assert_eq!(total, names.len(), "duplicate catalog entry");
    }

    #[test]
    fn dev_registry_contains_the_catalog_integrations() {
        // The regression inventory made possible: a registry that silently
        // lacks an integration because nothing linked its crate.
        let registry = oss_capability_registry_for_grade(DeploymentGrade::Dev);
        for plugin in capability_plugins() {
            if !capability_is_enabled(plugin, DeploymentGrade::Dev) {
                continue;
            }
            let id = (plugin.factory)().id().to_string();
            assert!(
                registry.has(&id),
                "{id} is enabled at dev grade but missing from the registry"
            );
        }
    }

    #[test]
    fn experimental_capabilities_stay_out_of_prod() {
        for plugin in capability_plugins() {
            if plugin.experimental_only {
                assert!(
                    !capability_is_enabled(plugin, DeploymentGrade::Prod),
                    "experimental capability registered at prod grade"
                );
            }
        }
    }
}
