#![allow(deprecated)]

use everruns_capabilities::integrations_catalog as canonical;
use everruns_integrations_catalog::{CATALOG, capability_plugins, oss_capability_registry};

#[test]
fn old_catalog_path_preserves_the_hosted_registry() {
    assert!(std::ptr::eq(CATALOG, canonical::CATALOG));
    assert_eq!(
        capability_plugins().count(),
        canonical::capability_plugins().count()
    );
    let registry = oss_capability_registry();
    let canonical_registry = canonical::oss_capability_registry();
    for plugin in capability_plugins() {
        let id = (plugin.factory)().id().to_string();
        let expected = canonical_registry.has(&id);
        assert_eq!(registry.has(&id), expected, "{id}");
    }
}
