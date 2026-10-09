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
//! // Integrations behind an off-by-default feature flag stay out of a production registry.
//! for plugin in everruns_integrations_catalog::capability_plugins() {
//!     if plugin.feature_flag == Some("sprites") {
//!         assert!(!capability_is_enabled(plugin, DeploymentGrade::Prod));
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

use everruns_contracts::connector::{ConnectorPlugin, ConnectorRegistry};
use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_core::{DeploymentGrade, FeatureFlagDefinition};
use std::collections::HashMap;
use std::sync::LazyLock;

/// One integration crate's contribution to the hosted product.
pub struct CatalogEntry {
    /// The contributing crate, for diagnostics and for embedders filtering the
    /// catalog. Feature modules of `everruns-integrations` name the module too
    /// (`everruns-integrations::modal`), since one crate contributes several.
    pub crate_name: &'static str,
    /// Capabilities the crate contributes.
    pub capabilities: &'static [IntegrationPlugin],
    /// Connectors the crate contributes.
    pub connectors: &'static [ConnectorPlugin],
    /// Feature flags the crate declares for its plugins.
    pub feature_flags: &'static [FeatureFlagDefinition],
}

/// Every integration composed into the hosted product, in registration order.
///
/// Registration order decides which contributor wins a canonical-id collision:
/// [`CapabilityRegistry::register_plugins`] and [`ConnectorRegistry`] both let
/// a later entry replace an earlier one.
pub const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        crate_name: "everruns-integrations-experimental::agentid",
        capabilities: everruns_integrations_experimental::agentid::CAPABILITY_PLUGINS,
        connectors: everruns_integrations_experimental::agentid::CONNECTOR_PLUGINS,
        feature_flags: everruns_integrations_experimental::agentid::FEATURE_FLAGS,
    },
    CatalogEntry {
        crate_name: "everruns-ard",
        capabilities: everruns_ard::CAPABILITY_PLUGINS,
        connectors: everruns_ard::CONNECTOR_PLUGINS,
        feature_flags: everruns_ard::FEATURE_FLAGS,
    },
    CatalogEntry {
        crate_name: "everruns-integrations::bashkit",
        capabilities: everruns_integrations::bashkit::tools_in_shell::CAPABILITY_PLUGINS,
        connectors: &[],
        feature_flags: everruns_integrations::bashkit::tools_in_shell::FEATURE_FLAGS,
    },
    CatalogEntry {
        crate_name: "everruns-integrations::brave_search",
        capabilities: everruns_integrations::brave_search::CAPABILITY_PLUGINS,
        connectors: everruns_integrations::brave_search::CONNECTOR_PLUGINS,
        feature_flags: everruns_integrations::brave_search::FEATURE_FLAGS,
    },
    CatalogEntry {
        crate_name: "everruns-integrations::browserless",
        capabilities: everruns_integrations::browserless::CAPABILITY_PLUGINS,
        connectors: everruns_integrations::browserless::CONNECTOR_PLUGINS,
        feature_flags: everruns_integrations::browserless::FEATURE_FLAGS,
    },
    CatalogEntry {
        crate_name: "everruns-integrations::cursor",
        capabilities: everruns_integrations::cursor::CAPABILITY_PLUGINS,
        connectors: everruns_integrations::cursor::CONNECTOR_PLUGINS,
        feature_flags: &[],
    },
    CatalogEntry {
        crate_name: "everruns-integrations::daytona",
        capabilities: everruns_integrations::daytona::CAPABILITY_PLUGINS,
        connectors: everruns_integrations::daytona::CONNECTOR_PLUGINS,
        feature_flags: everruns_integrations::daytona::FEATURE_FLAGS,
    },
    CatalogEntry {
        crate_name: "everruns-integrations-experimental::deno",
        capabilities: everruns_integrations_experimental::deno::CAPABILITY_PLUGINS,
        connectors: everruns_integrations_experimental::deno::CONNECTOR_PLUGINS,
        feature_flags: everruns_integrations_experimental::deno::FEATURE_FLAGS,
    },
    CatalogEntry {
        crate_name: "everruns-integrations::docker",
        capabilities: everruns_integrations::docker::CAPABILITY_PLUGINS,
        connectors: &[],
        feature_flags: &[],
    },
    CatalogEntry {
        crate_name: "everruns-integrations::duckduckgo",
        capabilities: everruns_integrations::duckduckgo::CAPABILITY_PLUGINS,
        connectors: &[],
        feature_flags: everruns_integrations::duckduckgo::FEATURE_FLAGS,
    },
    CatalogEntry {
        crate_name: "everruns-integrations::e2b",
        capabilities: everruns_integrations::e2b::CAPABILITY_PLUGINS,
        connectors: everruns_integrations::e2b::CONNECTOR_PLUGINS,
        feature_flags: everruns_integrations::e2b::FEATURE_FLAGS,
    },
    CatalogEntry {
        crate_name: "everruns-integrations::github",
        capabilities: everruns_integrations::github::CAPABILITY_PLUGINS,
        connectors: &[],
        feature_flags: &[],
    },
    CatalogEntry {
        crate_name: "everruns-integrations::modal",
        capabilities: everruns_integrations::modal::CAPABILITY_PLUGINS,
        connectors: everruns_integrations::modal::CONNECTOR_PLUGINS,
        feature_flags: everruns_integrations::modal::FEATURE_FLAGS,
    },
    CatalogEntry {
        crate_name: "everruns-integrations::openai_image",
        capabilities: everruns_integrations::openai_image::CAPABILITY_PLUGINS,
        connectors: &[],
        feature_flags: &[],
    },
    CatalogEntry {
        crate_name: "everruns-integrations::parallel",
        capabilities: everruns_integrations::parallel::CAPABILITY_PLUGINS,
        connectors: everruns_integrations::parallel::CONNECTOR_PLUGINS,
        feature_flags: everruns_integrations::parallel::FEATURE_FLAGS,
    },
    CatalogEntry {
        crate_name: "everruns-integrations-experimental::sprites",
        capabilities: everruns_integrations_experimental::sprites::CAPABILITY_PLUGINS,
        connectors: everruns_integrations_experimental::sprites::CONNECTOR_PLUGINS,
        feature_flags: everruns_integrations_experimental::sprites::FEATURE_FLAGS,
    },
    CatalogEntry {
        crate_name: "everruns-integrations::typesafe",
        capabilities: everruns_integrations::typesafe::CAPABILITY_PLUGINS,
        connectors: everruns_integrations::typesafe::CONNECTOR_PLUGINS,
        feature_flags: everruns_integrations::typesafe::FEATURE_FLAGS,
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

/// Every feature flag the catalog's integrations declare.
///
/// Decision: integration availability is an ordinary feature flag rather than
/// a separate dev-only switch on each plugin. A flag gets rollout grades,
/// `FEATURE_<NAME>` overrides, and organisation opt-in like every other flag,
/// and the hosted platform lists these in its settings. Each integration crate
/// owns its flags' default grades next to the plugins they gate. Plugins may
/// also name a platform execution flag (`docker_capability`,
/// `container_sandbox`, `machine_payments`), whose defaults live in
/// `everruns_core::execution_features`.
pub fn feature_flag_definitions() -> impl Iterator<Item = &'static FeatureFlagDefinition> {
    CATALOG.iter().flat_map(|entry| entry.feature_flags.iter())
}

/// Whether `flag` is available on a `grade` deployment under the current
/// environment. Unknown flags fail closed.
///
/// Availability only decides registration; the hosted platform still filters
/// by each organisation's effective flags before a capability or connector is
/// used.
pub fn feature_flag_available(flag: &str, grade: DeploymentGrade) -> bool {
    everruns_core::feature_flag_available(flag, feature_flag_definitions(), grade)
}

/// Whether a capability plugin is registered for `grade` under the current environment.
pub fn capability_is_enabled(plugin: &IntegrationPlugin, grade: DeploymentGrade) -> bool {
    plugin
        .feature_flag
        .is_none_or(|flag| feature_flag_available(flag, grade))
}

/// Whether a connector plugin is registered for `grade` under the current environment.
pub fn connector_is_enabled(plugin: &ConnectorPlugin, grade: DeploymentGrade) -> bool {
    plugin
        .feature_flag
        .is_none_or(|flag| feature_flag_available(flag, grade))
}

/// Register the catalog's capabilities accepted by `grade` onto `registry`.
pub fn register_capabilities(registry: &mut CapabilityRegistry, grade: DeploymentGrade) {
    registry.register_plugins(capability_plugins(), |plugin| {
        capability_is_enabled(plugin, grade)
    });
}

/// The default OSS capability registry: portable builtins, then this catalog's
/// integrations, then the hosted product capabilities.
///
/// Decision: integrations register between builtins and hosted capabilities so
/// a hosted capability still wins a canonical-id collision, which is the order
/// the inventory-based composition had.
pub fn oss_capability_registry_for_grade(grade: DeploymentGrade) -> CapabilityRegistry {
    let mut registry = everruns_capabilities::capabilities::portable_capability_registry();
    register_capabilities(&mut registry, grade);
    everruns_capabilities::capabilities::register_hosted_capabilities(&mut registry, grade);
    registry
}

/// [`oss_capability_registry_for_grade`] using the environment grade.
pub fn oss_capability_registry() -> CapabilityRegistry {
    oss_capability_registry_for_grade(DeploymentGrade::from_env())
}

/// Register the catalog's connectors accepted by `grade` onto `registry`.
pub fn register_connectors(registry: &mut ConnectorRegistry, grade: DeploymentGrade) {
    for plugin in connector_plugins() {
        if connector_is_enabled(plugin, grade) {
            registry.register_boxed((plugin.factory)());
        }
    }
}

static CAPABILITY_FLAGS: LazyLock<HashMap<String, &'static str>> = LazyLock::new(|| {
    capability_plugins()
        .filter_map(|plugin| Some(((plugin.factory)().id().to_string(), plugin.feature_flag?)))
        .collect()
});

static CONNECTOR_FLAGS: LazyLock<HashMap<String, &'static str>> = LazyLock::new(|| {
    connector_plugins()
        .filter_map(|plugin| {
            Some((
                (plugin.factory)().provider_id().to_string(),
                plugin.feature_flag?,
            ))
        })
        .collect()
});

/// The feature flag gating a catalog capability, by capability id.
pub fn capability_feature_flag(capability_id: &str) -> Option<&'static str> {
    CAPABILITY_FLAGS.get(capability_id).copied()
}

/// The feature flag gating a catalog connector, by provider id.
pub fn connector_feature_flag(provider_id: &str) -> Option<&'static str> {
    CONNECTOR_FLAGS.get(provider_id).copied()
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

    /// Flags the platform defines in `everruns_core::execution_features`.
    const PLATFORM_FLAGS: &[&str] = &["docker_capability", "container_sandbox", "machine_payments"];

    fn named_flags() -> Vec<&'static str> {
        capability_plugins()
            .filter_map(|plugin| plugin.feature_flag)
            .chain(connector_plugins().filter_map(|plugin| plugin.feature_flag))
            .collect()
    }

    fn definition(flag: &str) -> Option<&'static FeatureFlagDefinition> {
        feature_flag_definitions().find(|definition| definition.name == flag)
    }

    #[test]
    fn every_named_flag_is_defined() {
        for flag in named_flags() {
            assert!(
                PLATFORM_FLAGS.contains(&flag) || definition(flag).is_some(),
                "{flag} is named by a plugin but has no definition"
            );
        }
    }

    #[test]
    fn every_defined_flag_gates_something_and_names_are_unique() {
        let named = named_flags();
        let mut names: Vec<_> = feature_flag_definitions().map(|d| d.name).collect();
        for name in &names {
            assert!(named.contains(name), "{name} is defined but gates nothing");
            assert!(
                !PLATFORM_FLAGS.contains(name),
                "{name} shadows a platform flag"
            );
        }
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(total, names.len(), "duplicate integration flag");
    }

    #[test]
    fn dev_grade_flags_stay_out_of_prod() {
        for plugin in capability_plugins() {
            let Some(flag) = plugin.feature_flag else {
                continue;
            };
            let Some(definition) = definition(flag) else {
                continue;
            };
            if definition.grade == everruns_core::FeatureFlagGrade::Dev
                && std::env::var(format!("FEATURE_{}", flag.to_ascii_uppercase())).is_err()
            {
                assert!(
                    capability_is_enabled(plugin, DeploymentGrade::Dev),
                    "{flag}"
                );
                assert!(
                    !capability_is_enabled(plugin, DeploymentGrade::Prod),
                    "{flag}"
                );
            }
        }
    }

    #[test]
    fn daytona_capabilities_and_connection_are_ungated() {
        assert_eq!(capability_feature_flag("daytona"), None);
        assert_eq!(connector_feature_flag("daytona"), None);
        assert_eq!(connector_feature_flag("e2b"), None);
        assert_eq!(capability_feature_flag("modal"), Some("modal"));
    }

    #[test]
    fn prod_connector_registry_drops_flagged_connectors() {
        let mut dev = ConnectorRegistry::new();
        register_connectors(&mut dev, DeploymentGrade::Dev);
        let mut prod = ConnectorRegistry::new();
        register_connectors(&mut prod, DeploymentGrade::Prod);
        assert!(prod.has("e2b"));
        assert!(prod.has("daytona"));
        if std::env::var("FEATURE_BROWSERLESS").is_err() {
            assert!(prod.has("browserless"), "browserless is adoption grade");
        }
        if std::env::var("FEATURE_DENO").is_err() {
            assert!(!dev.has("deno"), "deno is off by default");
            assert!(!prod.has("deno"), "deno is off by default");
        }
    }
}
