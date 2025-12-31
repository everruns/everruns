// Unit tests for OpenAI provider

#[cfg(test)]
mod driver_tests {
    use crate::{register_driver, DriverRegistry, OpenAILlmDriver};
    use everruns_core::llm_driver_registry::{ProviderConfig, ProviderType};

    #[test]
    fn test_driver_with_api_key() {
        let driver = OpenAILlmDriver::new("test-key");
        // Just verify it can be created
        assert!(format!("{:?}", driver).contains("OpenAILlmDriver"));
    }

    #[test]
    fn test_driver_with_base_url() {
        let driver =
            OpenAILlmDriver::with_base_url("test-key", "https://custom.api.com/v1/completions");
        assert!(format!("{:?}", driver).contains("OpenAILlmDriver"));
        assert_eq!(driver.api_url(), "https://custom.api.com/v1/completions");
    }

    #[test]
    fn test_register_driver() {
        let mut registry = DriverRegistry::new();
        assert!(!registry.has_driver(&ProviderType::OpenAI));
        assert!(!registry.has_driver(&ProviderType::AzureOpenAI));

        register_driver(&mut registry);

        assert!(registry.has_driver(&ProviderType::OpenAI));
        assert!(registry.has_driver(&ProviderType::AzureOpenAI));

        // Verify drivers can be created via registry
        let config = ProviderConfig::new(ProviderType::OpenAI).with_api_key("test-key");
        let driver = registry.create_driver(&config);
        assert!(driver.is_ok());

        let azure_config = ProviderConfig::new(ProviderType::AzureOpenAI).with_api_key("test-key");
        let azure_driver = registry.create_driver(&azure_config);
        assert!(azure_driver.is_ok());
    }
}

#[cfg(test)]
mod provider_tests {
    use crate::types::{ChatMessage, LlmConfig, MessageRole};

    #[test]
    fn test_chat_message_creation() {
        let msg = ChatMessage {
            role: MessageRole::User,
            content: "Hello".to_string(),
            tool_calls: None,
            tool_call_id: None,
        };
        assert_eq!(msg.content, "Hello");
        assert!(matches!(msg.role, MessageRole::User));
    }

    #[test]
    fn test_message_role_variants() {
        let system = MessageRole::System;
        let user = MessageRole::User;
        let assistant = MessageRole::Assistant;

        // Ensure all roles are distinct
        assert!(matches!(system, MessageRole::System));
        assert!(matches!(user, MessageRole::User));
        assert!(matches!(assistant, MessageRole::Assistant));
    }

    #[test]
    fn test_llm_config_with_all_options() {
        let config = LlmConfig {
            model: "gpt-5.2".to_string(),
            temperature: Some(0.7),
            max_tokens: Some(1000),
            system_prompt: Some("You are helpful".to_string()),
            tools: Vec::new(),
        };

        assert_eq!(config.model, "gpt-5.2");
        assert_eq!(config.temperature, Some(0.7));
        assert_eq!(config.max_tokens, Some(1000));
        assert_eq!(config.system_prompt, Some("You are helpful".to_string()));
    }

    #[test]
    fn test_llm_config_minimal() {
        let config = LlmConfig {
            model: "gpt-3.5-turbo".to_string(),
            temperature: None,
            max_tokens: None,
            system_prompt: None,
            tools: Vec::new(),
        };

        assert_eq!(config.model, "gpt-3.5-turbo");
        assert!(config.temperature.is_none());
        assert!(config.max_tokens.is_none());
        assert!(config.system_prompt.is_none());
    }

    #[test]
    fn test_message_serialization() {
        let msg = ChatMessage {
            role: MessageRole::User,
            content: "Test message".to_string(),
            tool_calls: None,
            tool_call_id: None,
        };

        let json = serde_json::to_string(&msg).expect("Failed to serialize");
        assert!(json.contains("user"));
        assert!(json.contains("Test message"));

        let deserialized: ChatMessage = serde_json::from_str(&json).expect("Failed to deserialize");
        assert_eq!(deserialized.content, msg.content);
    }

    #[test]
    fn test_config_serialization() {
        let config = LlmConfig {
            model: "gpt-5.2".to_string(),
            temperature: Some(0.5),
            max_tokens: Some(500),
            system_prompt: None,
            tools: Vec::new(),
        };

        let json = serde_json::to_string(&config).expect("Failed to serialize");
        let deserialized: LlmConfig = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(deserialized.model, config.model);
        assert_eq!(deserialized.temperature, config.temperature);
        assert_eq!(deserialized.max_tokens, config.max_tokens);
    }

    #[test]
    fn test_temperature_bounds() {
        // Test valid temperature range
        let config_low = LlmConfig {
            model: "test".to_string(),
            temperature: Some(0.0),
            max_tokens: None,
            system_prompt: None,
            tools: Vec::new(),
        };
        assert_eq!(config_low.temperature, Some(0.0));

        let config_high = LlmConfig {
            model: "test".to_string(),
            temperature: Some(2.0),
            max_tokens: None,
            system_prompt: None,
            tools: Vec::new(),
        };
        assert_eq!(config_high.temperature, Some(2.0));
    }

    #[test]
    fn test_system_prompt_in_config() {
        // Decision: System prompt can be in config OR as first message
        // This test validates the config approach
        let config = LlmConfig {
            model: "test".to_string(),
            temperature: None,
            max_tokens: None,
            system_prompt: Some("You are a comedian".to_string()),
            tools: Vec::new(),
        };

        assert!(config.system_prompt.is_some());
        assert_eq!(config.system_prompt.unwrap(), "You are a comedian");
    }

    #[test]
    fn test_message_conversation_flow() {
        // Decision: Messages maintain conversation order
        let messages = [
            ChatMessage {
                role: MessageRole::System,
                content: "You are helpful".to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: MessageRole::User,
                content: "Hello".to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: MessageRole::Assistant,
                content: "Hi there!".to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: MessageRole::User,
                content: "How are you?".to_string(),
                tool_calls: None,
                tool_call_id: None,
            },
        ];

        assert_eq!(messages.len(), 4);
        assert!(matches!(messages[0].role, MessageRole::System));
        assert!(matches!(messages[1].role, MessageRole::User));
        assert!(matches!(messages[2].role, MessageRole::Assistant));
        assert!(matches!(messages[3].role, MessageRole::User));
    }

    #[test]
    fn test_chat_message_to_openai_conversion() {
        let msg = ChatMessage {
            role: MessageRole::User,
            content: "Hello world".to_string(),
            tool_calls: None,
            tool_call_id: None,
        };

        let openai_msg = msg.to_openai();
        assert_eq!(openai_msg.role, "user");
        assert_eq!(openai_msg.content, Some("Hello world".to_string()));
        assert!(openai_msg.tool_calls.is_none());
    }

    #[test]
    fn test_chat_message_to_openai_all_roles() {
        let roles = [
            (MessageRole::System, "system"),
            (MessageRole::User, "user"),
            (MessageRole::Assistant, "assistant"),
            (MessageRole::Tool, "tool"),
        ];

        for (role, expected_str) in roles {
            let msg = ChatMessage {
                role,
                content: "test".to_string(),
                tool_calls: None,
                tool_call_id: None,
            };
            let openai_msg = msg.to_openai();
            assert_eq!(openai_msg.role, expected_str);
        }
    }
}
