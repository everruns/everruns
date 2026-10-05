use super::*;
use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::typed_id::SessionId;
use everruns_contracts::runtime::connection_services::ProviderCredentialStore;
use everruns_contracts::runtime::connection_services::ProviderCredentials;
use std::sync::Arc;

struct MockProviderCredentialStore {
    providers: Vec<(&'static str, ProviderCredentials)>,
}

#[async_trait]
impl ProviderCredentialStore for MockProviderCredentialStore {
    async fn get_default_provider_credentials(
        &self,
        provider_type: &str,
    ) -> Result<Option<ProviderCredentials>> {
        Ok(self
            .providers
            .iter()
            .find(|(name, _)| *name == provider_type)
            .map(|(_, credentials)| credentials.clone()))
    }
}

#[test]
fn normalize_workspace_paths() {
    assert_eq!(normalize_workspace_path("/workspace/out"), "/out");
    assert_eq!(normalize_workspace_path("out/file.png"), "/out/file.png");
}

#[test]
fn output_filename_indexes_multiple_images() {
    assert_eq!(output_filename("image", 0, 1, "png"), "image.png");
    assert_eq!(output_filename("image", 1, 3, "jpeg"), "image-2.jpg");
}

#[test]
fn parse_image_id_strings() {
    let id = ImageId::new();
    let value = json!({ "prompt": "edit", "image_id": id.to_string() });
    let args: EditImageArgs = serde_json::from_value(value).unwrap();
    assert_eq!(args.image_id, Some(id));
}

#[test]
fn capability_declares_session_file_system_dependency() {
    let capability = GptImageGenCapability;
    assert_eq!(capability.dependencies(), vec!["session_file_system"]);
}

// With no OpenAI provider configured, credentials must fall back to the
// org's default Azure OpenAI provider: its base URL is validated to the
// OpenAI-compatible `/openai/v1` surface, which serves the Images API.
#[tokio::test]
async fn falls_back_to_azure_openai_provider_credentials() {
    let session_id = SessionId::new();
    let provider = Arc::new(MockProviderCredentialStore {
        providers: vec![(
            "azure_openai",
            ProviderCredentials {
                api_key: "azure-key".to_string(),
                base_url: Some("https://res.openai.azure.com/openai/v1".to_string()),
            },
        )],
    });
    let context = ToolContext {
        session_id,
        storage_store: None,
        provider_credential_store: Some(provider),
        ..ToolContext::new(session_id)
    };

    let resolved = resolve_client_config(
        &context,
        ImageGenerationModel::GptImage2,
        ImageModelFallback::Auto,
    )
    .await
    .expect("resolve config");

    assert_eq!(resolved.api_key, "azure-key");
    assert_eq!(
        resolved.base_url,
        Some("https://res.openai.azure.com/openai/v1".to_string())
    );
}

#[tokio::test]
async fn openai_provider_wins_over_azure_openai_provider() {
    let session_id = SessionId::new();
    let provider = Arc::new(MockProviderCredentialStore {
        providers: vec![
            (
                "openai",
                ProviderCredentials {
                    api_key: "openai-key".to_string(),
                    base_url: None,
                },
            ),
            (
                "azure_openai",
                ProviderCredentials {
                    api_key: "azure-key".to_string(),
                    base_url: Some("https://res.openai.azure.com/openai/v1".to_string()),
                },
            ),
        ],
    });
    let context = ToolContext {
        session_id,
        storage_store: None,
        provider_credential_store: Some(provider),
        ..ToolContext::new(session_id)
    };

    let resolved = resolve_client_config(
        &context,
        ImageGenerationModel::GptImage2,
        ImageModelFallback::Auto,
    )
    .await
    .expect("resolve config");

    assert_eq!(resolved.api_key, "openai-key");
}

fn mock_context_with_named_providers(
    providers: Vec<(&'static str, ProviderCredentials)>,
) -> ToolContext {
    let session_id = SessionId::new();
    let provider = Arc::new(MockProviderCredentialStore { providers });
    ToolContext {
        session_id,
        storage_store: None,
        provider_credential_store: Some(provider),
        ..ToolContext::new(session_id)
    }
}

#[tokio::test]
async fn muse_model_resolves_meta_provider() {
    let context = mock_context_with_named_providers(vec![(
        "meta",
        ProviderCredentials {
            api_key: "meta-key".to_string(),
            base_url: None,
        },
    )]);
    let resolved = resolve_client_config(
        &context,
        ImageGenerationModel::MuseImage,
        ImageModelFallback::Auto,
    )
    .await
    .expect("resolve config");
    assert_eq!(resolved.provider_type, "meta");
    assert_eq!(resolved.api_key, "meta-key");
    assert_eq!(resolved.base_url.as_deref(), Some(DEFAULT_META_BASE_URL));
    assert_eq!(resolved.api_model, "muse-image-1.0");
}

#[tokio::test]
async fn muse_model_resolves_openrouter_provider_with_openrouter_model_id() {
    let context = mock_context_with_named_providers(vec![(
        "openrouter",
        ProviderCredentials {
            api_key: "or-key".to_string(),
            base_url: None,
        },
    )]);
    let resolved = resolve_client_config(
        &context,
        ImageGenerationModel::MuseImage,
        ImageModelFallback::Auto,
    )
    .await
    .expect("resolve config");
    assert_eq!(resolved.provider_type, "openrouter");
    assert_eq!(
        resolved.base_url.as_deref(),
        Some(DEFAULT_OPENROUTER_BASE_URL)
    );
    assert_eq!(resolved.api_model, "meta/muse-image");
}

#[tokio::test]
async fn muse_model_prefers_meta_provider_over_openrouter() {
    let context = mock_context_with_named_providers(vec![
        (
            "openrouter",
            ProviderCredentials {
                api_key: "or-key".to_string(),
                base_url: None,
            },
        ),
        (
            "meta",
            ProviderCredentials {
                api_key: "meta-key".to_string(),
                base_url: None,
            },
        ),
    ]);
    let resolved = resolve_client_config(
        &context,
        ImageGenerationModel::MuseImage,
        ImageModelFallback::Auto,
    )
    .await
    .expect("resolve config");
    assert_eq!(resolved.api_key, "meta-key");
    assert_eq!(resolved.api_model, "muse-image-1.0");
}

#[tokio::test]
async fn muse_model_ignores_openai_provider() {
    let context = mock_context_with_named_providers(vec![(
        "openai",
        ProviderCredentials {
            api_key: "openai-key".to_string(),
            base_url: None,
        },
    )]);
    let error = resolve_client_config(
        &context,
        ImageGenerationModel::MuseImage,
        ImageModelFallback::Auto,
    )
    .await
    .unwrap_err();
    let ToolExecutionResult::ToolError(message) = error else {
        panic!("expected tool error, got {error:?}");
    };
    assert!(message.contains("Image credentials are not configured"));
}

#[tokio::test]
async fn gpt_model_falls_back_to_muse_when_only_muse_providers_exist() {
    let context = mock_context_with_named_providers(vec![(
        "openrouter",
        ProviderCredentials {
            api_key: "or-key".to_string(),
            base_url: None,
        },
    )]);
    let resolved = resolve_client_config(
        &context,
        ImageGenerationModel::GptImage2,
        ImageModelFallback::Auto,
    )
    .await
    .expect("resolve config");
    assert_eq!(resolved.provider_type, "openrouter");
    assert_eq!(resolved.api_model, "meta/muse-image");
}

#[tokio::test]
async fn gpt_model_prefers_openai_over_muse_fallback() {
    let context = mock_context_with_named_providers(vec![
        (
            "meta",
            ProviderCredentials {
                api_key: "meta-key".to_string(),
                base_url: None,
            },
        ),
        (
            "openai",
            ProviderCredentials {
                api_key: "openai-key".to_string(),
                base_url: None,
            },
        ),
    ]);
    let resolved = resolve_client_config(
        &context,
        ImageGenerationModel::GptImage2,
        ImageModelFallback::Auto,
    )
    .await
    .expect("resolve config");
    assert_eq!(resolved.provider_type, "openai");
    assert_eq!(resolved.api_model, "gpt-image-2");
}

#[tokio::test]
async fn gpt_model_with_fallback_off_errors_when_only_muse_providers_exist() {
    let context = mock_context_with_named_providers(vec![(
        "openrouter",
        ProviderCredentials {
            api_key: "or-key".to_string(),
            base_url: None,
        },
    )]);
    let error = resolve_client_config(
        &context,
        ImageGenerationModel::GptImage2,
        ImageModelFallback::Off,
    )
    .await
    .unwrap_err();
    let ToolExecutionResult::ToolError(message) = error else {
        panic!("expected tool error, got {error:?}");
    };
    assert!(message.contains("Image credentials are not configured"));
}

#[test]
fn capability_config_defaults_to_auto_fallback() {
    let config = parse_capability_config(&json!({})).unwrap();
    assert_eq!(config.fallback, ImageModelFallback::Auto);
}

#[test]
fn capability_config_accepts_fallback_off() {
    let config = parse_capability_config(&json!({
        "fallback": "off"
    }))
    .unwrap();
    assert_eq!(config.fallback, ImageModelFallback::Off);
}

#[test]
fn capability_config_accepts_muse_image_model() {
    let config = parse_capability_config(&json!({
        "model": "muse-image-1.0"
    }))
    .unwrap();
    assert_eq!(config.model, ImageGenerationModel::MuseImage);
}

#[test]
fn capability_config_defaults_to_gpt_image_2() {
    let config = parse_capability_config(&json!({})).unwrap();
    assert_eq!(config.model, ImageGenerationModel::GptImage2);
    assert_eq!(config.default_quality, ImageGenerationQuality::Medium);
    assert_eq!(config.partial_images, 1);
}

#[test]
fn capability_config_accepts_legacy_model_override() {
    let config = parse_capability_config(&json!({
        "model": "gpt-image-1",
        "default_quality": "low",
        "partial_images": 2
    }))
    .unwrap();
    assert_eq!(config.model, ImageGenerationModel::GptImage1);
    assert_eq!(config.default_quality, ImageGenerationQuality::Low);
    assert_eq!(config.partial_images, 2);
}

#[test]
fn capability_config_ignores_unknown_fields() {
    let config = parse_capability_config(&json!({
        "model": "gpt-image-1",
        "default_quality": "high",
        "partial_images": 3,
        "future_field": true
    }))
    .unwrap();
    assert_eq!(config.model, ImageGenerationModel::GptImage1);
    assert_eq!(config.default_quality, ImageGenerationQuality::High);
    assert_eq!(config.partial_images, 3);
}

#[test]
fn capability_config_rejects_unknown_model_override() {
    let result = parse_capability_config(&json!({
        "model": "chatgpt-image-latest"
    }));
    let error = result.unwrap_err();
    match error {
        ToolExecutionResult::ToolError(message) => {
            assert!(message.contains("Invalid gpt_image_gen config"));
        }
        other => panic!("expected tool error, got {other:?}"),
    }
}

#[test]
fn capability_config_rejects_invalid_partial_image_count() {
    let result = parse_capability_config(&json!({
        "partial_images": 4
    }));
    let error = result.unwrap_err();
    match error {
        ToolExecutionResult::ToolError(message) => {
            assert!(message.contains("partial_images must be between 0 and 3"));
        }
        other => panic!("expected tool error, got {other:?}"),
    }
}

#[test]
fn resolve_quality_prefers_explicit_argument() {
    assert_eq!(
        resolve_quality(Some("high"), ImageGenerationQuality::Medium),
        "high"
    );
}

#[test]
fn resolve_quality_uses_capability_default_when_omitted() {
    assert_eq!(
        resolve_quality(None, ImageGenerationQuality::Medium),
        "medium"
    );
}

#[test]
fn resolve_partial_images_defaults_single_image_requests() {
    let config = GptImageGenCapabilityConfig::default();
    assert_eq!(resolve_partial_images(1, &config), Some(1));
}

#[test]
fn resolve_partial_images_disables_preview_streaming_for_batches() {
    let config = GptImageGenCapabilityConfig::default();
    assert_eq!(resolve_partial_images(3, &config), None);
}

#[test]
fn system_prompt_marks_image_tools_as_directly_invokable() {
    let prompt = GptImageGenCapability.system_prompt_addition().unwrap();
    assert!(prompt.contains("call them directly"));
    assert!(prompt.contains("do not claim they are unavailable"));
    assert!(prompt.contains("stop at writing prompts"));
}

#[tokio::test]
async fn image_system_prompt_within_budget() {
    let ctx = everruns_contracts::runtime::capabilities::SystemPromptContext::without_file_store(
        everruns_contracts::typed_id::SessionId::new(),
    );
    let prompt = GptImageGenCapability
        .system_prompt_contribution(&ctx)
        .await
        .unwrap();
    assert!(prompt.len() <= 850, "prompt is {} bytes", prompt.len());
}

#[test]
fn localizations_cover_schema_summary_and_uk_name() {
    let cap = GptImageGenCapability;
    assert!(cap.describe_schema(None).is_some());
    assert_ne!(cap.localized_name(Some("uk-UA")), cap.name());
}

#[test]
fn image_tools_are_never_deferred_for_tool_search() {
    for definition in GptImageGenCapability.tool_definitions() {
        let ToolDefinition::Builtin(builtin) = definition else {
            panic!("expected builtin tool definition");
        };
        assert_eq!(builtin.deferrable, DeferrablePolicy::Never);
    }
}
