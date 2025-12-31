// Unit tests for Anthropic driver

use crate::{register_driver, AnthropicLlmDriver, DriverRegistry};
use everruns_core::llm_driver_registry::{ProviderConfig, ProviderType};

#[test]
fn test_driver_with_api_key() {
    let driver = AnthropicLlmDriver::new("test-key");
    // Just verify it can be created
    assert!(format!("{:?}", driver).contains("AnthropicLlmDriver"));
}

#[test]
fn test_driver_with_base_url() {
    let driver =
        AnthropicLlmDriver::with_base_url("test-key", "https://custom.api.com/v1/messages");
    assert!(format!("{:?}", driver).contains("AnthropicLlmDriver"));
}

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
