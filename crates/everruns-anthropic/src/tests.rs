// Unit tests for Anthropic driver

use crate::AnthropicLlmDriver;

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
