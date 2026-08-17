// Unit tests for OpenAI provider

#[cfg(test)]
mod driver_tests {
    use crate::{
        DriverRegistry, OpenAIChatDriver, OpenAICompletionsChatDriver, azure_provider,
        completions_provider, provider, register_driver,
    };
    use everruns_provider::driver_registry::{DriverId, ProviderConfig};

    #[test]
    fn test_responses_driver_is_wire_only() {
        let driver = OpenAIChatDriver::new();
        // Just verify it can be created
        assert!(format!("{:?}", driver).contains("OpenAIChatDriver"));
        assert!(format!("{:?}", driver).contains("Open Responses"));
        let service = provider("openai", "test-key");
        assert_eq!(
            service.endpoint().url("responses").as_deref(),
            Some("https://api.openai.com/v1/responses")
        );
    }

    #[test]
    fn test_provider_accepts_custom_base_url() {
        let service = provider("custom", "test-key").base_url("https://custom.api.com/v1");
        assert_eq!(
            service.endpoint().url("responses").as_deref(),
            Some("https://custom.api.com/v1/responses")
        );
    }

    #[test]
    fn test_driver_normalizes_v1_base_url() {
        let service = provider("openai", "test-key");
        assert_eq!(
            service.endpoint().url("responses").as_deref(),
            Some("https://api.openai.com/v1/responses")
        );
    }

    #[test]
    fn test_driver_normalizes_azure_v1_base_url() {
        let service = azure_provider(
            "azure",
            "https://resource.openai.azure.com/openai/v1/",
            "test-key",
        );
        assert_eq!(
            service.endpoint().url("responses").as_deref(),
            Some("https://resource.openai.azure.com/openai/v1/responses")
        );
    }

    #[test]
    fn test_completions_driver_with_api_key() {
        let driver = OpenAICompletionsChatDriver::new();
        assert!(format!("{:?}", driver).contains("OpenAICompletionsChatDriver"));
        assert!(format!("{:?}", driver).contains("Chat Completions"));
        let service = completions_provider("openai-completions", "test-key");
        assert_eq!(
            service.endpoint().url("chat/completions").as_deref(),
            Some("https://api.openai.com/v1/chat/completions")
        );
    }

    #[test]
    fn test_completions_driver_with_base_url() {
        let service = completions_provider("custom", "test-key")
            .base_url("https://custom.api.com/v1/chat/completions");
        assert_eq!(
            service.endpoint().url("chat/completions").as_deref(),
            Some("https://custom.api.com/v1/chat/completions")
        );
    }

    #[test]
    fn test_register_driver() {
        let mut registry = DriverRegistry::new();
        assert!(!registry.has_driver(&DriverId::OpenAI));
        assert!(!registry.has_driver(&DriverId::AzureOpenAI));
        assert!(!registry.has_driver(&DriverId::OpenAICompletions));

        register_driver(&mut registry);

        assert!(registry.has_driver(&DriverId::OpenAI));
        assert!(registry.has_driver(&DriverId::AzureOpenAI));
        assert!(registry.has_driver(&DriverId::OpenAICompletions));
        // OpenRouter is registered by everruns-openrouter, not this crate.
        assert!(!registry.has_driver(&DriverId::OpenRouter));

        // Verify OpenAI driver can be created
        let config = ProviderConfig::new(DriverId::OpenAI).with_api_key("test-key");
        let driver = registry.create_chat_driver(&config);
        assert!(driver.is_ok());

        let azure_config = ProviderConfig::new(DriverId::AzureOpenAI)
            .with_api_key("test-key")
            .with_base_url("https://resource.openai.azure.com/openai/v1");
        let azure_driver = registry.create_chat_driver(&azure_config);
        assert!(azure_driver.is_ok());

        // Verify OpenAI Completions driver can be created
        let completions_config =
            ProviderConfig::new(DriverId::OpenAICompletions).with_api_key("test-key");
        let completions_driver = registry.create_chat_driver(&completions_config);
        assert!(completions_driver.is_ok());
    }
}

#[cfg(test)]
mod provider_tests {
    use crate::types::{ChatMessage, MessageRole};

    // Trivial serde round-trips of derive-only structs (`ChatMessage`,
    // `LlmConfig`) were removed: they only re-asserted fields set on the line
    // above and exercised no provider logic. Real request/response shaping is
    // covered by the conversion and wiremock tests below.

    #[test]
    fn test_empty_content_message() {
        // OpenAI accepts empty content (unlike Anthropic which rejects it)
        // This test documents that empty strings are allowed
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: String::new(),
            tool_calls: None,
            tool_call_id: None,
        };
        assert_eq!(msg.content, "");
        // Can serialize without error
        let json = serde_json::to_string(&msg).expect("Should serialize empty content");
        assert!(json.contains("\"content\":\"\""));
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

#[cfg(test)]
mod descriptor_tests {
    use crate::register_driver;
    use everruns_provider::driver_registry::{DriverId, DriverRegistry, ServiceKind};

    #[test]
    fn registered_descriptors_declare_services_and_credentials() {
        let mut registry = DriverRegistry::new();
        register_driver(&mut registry);

        // OpenAI providers also power realtime voice sessions.
        let openai = registry.descriptor(&DriverId::OpenAI).unwrap();
        assert_eq!(openai.display_name, "OpenAI");
        assert!(openai.supports(ServiceKind::Chat));
        assert!(openai.supports(ServiceKind::Realtime));
        assert_eq!(openai.credential_schema.fields[0].name, "api_key");

        // The other OpenAI-protocol drivers are chat-only.
        for id in [DriverId::AzureOpenAI, DriverId::OpenAICompletions] {
            let descriptor = registry.descriptor(&id).unwrap();
            assert_eq!(descriptor.services, vec![ServiceKind::Chat]);
            assert_eq!(descriptor.credential_schema.fields[0].name, "api_key");
        }
    }
}
