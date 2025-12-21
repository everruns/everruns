// Unit tests for Anthropic provider

use crate::AnthropicProvider;

#[test]
fn test_provider_with_api_key() {
    let provider = AnthropicProvider::with_api_key("test-key".to_string());
    // Just verify it can be created
    assert!(format!("{:?}", provider).contains("AnthropicProvider"));
}

#[test]
fn test_provider_with_base_url() {
    let provider = AnthropicProvider::with_base_url(
        "test-key".to_string(),
        "https://custom.api.com/v1/messages".to_string(),
    );
    assert!(format!("{:?}", provider).contains("AnthropicProvider"));
}
