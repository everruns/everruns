// Unit tests for Anthropic driver

use crate::AnthropicDriver;

#[test]
fn test_driver_with_api_key() {
    let driver = AnthropicDriver::with_api_key("test-key".to_string());
    // Just verify it can be created
    assert!(format!("{:?}", driver).contains("AnthropicDriver"));
}

#[test]
fn test_driver_with_base_url() {
    let driver = AnthropicDriver::with_base_url(
        "test-key".to_string(),
        "https://custom.api.com/v1/messages".to_string(),
    );
    assert!(format!("{:?}", driver).contains("AnthropicDriver"));
}
