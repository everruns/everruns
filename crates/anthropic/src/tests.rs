// Unit tests for Anthropic driver

use crate::{DriverRegistry, register_driver};
use everruns_core::llm_driver_registry::{ProviderConfig, ProviderType};

#[test]
fn test_register_driver() {
    let mut registry = DriverRegistry::new();
    assert!(!registry.has_driver(&ProviderType::Anthropic));

    register_driver(&mut registry);

    assert!(registry.has_driver(&ProviderType::Anthropic));

    // Verify driver can be created via registry
    let config = ProviderConfig::new(ProviderType::Anthropic).with_api_key("test-key");
    let driver = registry.create_driver(&config);
    assert!(driver.is_ok());
}
