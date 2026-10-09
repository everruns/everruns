#![doc = include_str!("../README.md")]
#![deprecated(
    since = "0.45.0",
    note = "use everruns_capabilities::integrations_catalog with the hosted-integration-catalog feature"
)]
#![deny(missing_docs)]

#[deprecated(
    since = "0.45.0",
    note = "use everruns_capabilities::integrations_catalog"
)]
pub use everruns_capabilities::integrations_catalog::{
    CATALOG, CatalogEntry, capability_feature_flag, capability_is_enabled, capability_plugins,
    connector_feature_flag, connector_is_enabled, connector_plugins, feature_flag_available,
    feature_flag_definitions, oss_capability_registry, oss_capability_registry_for_grade,
    register_capabilities, register_connectors,
};
