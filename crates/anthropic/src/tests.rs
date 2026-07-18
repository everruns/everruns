// Unit tests for Anthropic driver

use crate::{DriverRegistry, register_driver};
use everruns_provider::driver_registry::{DriverId, ProviderConfig};

#[test]
fn test_register_driver() {
    let mut registry = DriverRegistry::new();
    assert!(!registry.has_driver(&DriverId::Anthropic));

    register_driver(&mut registry);

    assert!(registry.has_driver(&DriverId::Anthropic));

    // Verify driver can be created via registry
    let config = ProviderConfig::new(DriverId::Anthropic).with_api_key("test-key");
    let driver = registry.create_chat_driver(&config);
    assert!(driver.is_ok());
}
