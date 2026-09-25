use super::*;

// ========================================================================
// Profile merge tests
// ========================================================================

fn test_cost(
    input: f64,
    output: f64,
    cache_read: Option<f64>,
) -> everruns_provider::model::ModelCost {
    let mut cost = everruns_provider::model::ModelCost::new(input, output);
    cost.cache_read = cache_read;
    cost
}

fn base_profile() -> ModelProfile {
    ModelProfile {
        name: "Test".into(),
        family: "test".into(),
        description: None,
        release_date: None,
        last_updated: None,
        attachment: true,
        reasoning: false,
        temperature: true,
        knowledge: None,
        tool_call: true,
        structured_output: true,
        open_weights: false,
        cost: None,
        limits: None,
        modalities: None,
        reasoning_effort: None,
        speed: None,
        verbosity: None,
        tool_search: false,
        supported_parameters: Vec::new(),
        supports_phases: false,
        supports_server_compaction: false,
    }
}

#[test]
fn merge_hardcoded_wins_for_curated_fields() {
    let hardcoded = ModelProfile {
        name: "Hardcoded Name".into(),
        family: "hardcoded-family".into(),
        cost: Some(test_cost(5.0, 25.0, None)),
        ..base_profile()
    };
    let discovered = ModelProfile {
        name: "Discovered Name".into(),
        family: "discovered-family".into(),
        knowledge: Some("2025-01-01".into()),
        cost: Some(test_cost(0.5, 1.0, Some(0.1))),
        ..base_profile()
    };

    let merged = ModelService::merge_profiles(hardcoded, discovered);
    assert_eq!(merged.name, "Hardcoded Name");
    assert_eq!(merged.family, "hardcoded-family");
    assert_eq!(merged.knowledge.as_deref(), Some("2025-01-01"));
    assert_eq!(merged.cost.unwrap().input, 5.0);
}

#[test]
fn merge_discovered_fills_gaps() {
    use everruns_provider::model::ModelLimits;

    let hardcoded = ModelProfile {
        limits: None,
        ..base_profile()
    };
    let discovered = ModelProfile {
        limits: Some(ModelLimits {
            context: 200_000,
            input: None,
            output: 64_000,
            max_media: None,
        }),
        knowledge: Some("2025-02-01".into()),
        cost: Some(test_cost(0.5, 1.0, Some(0.1))),
        supported_parameters: vec!["tools".into(), "temperature".into()],
        ..base_profile()
    };

    let merged = ModelService::merge_profiles(hardcoded, discovered);
    assert!(merged.limits.is_some());
    assert_eq!(merged.limits.unwrap().context, 200_000);
    assert_eq!(merged.knowledge.as_deref(), Some("2025-02-01"));
    assert_eq!(merged.cost.unwrap().output, 1.0);
    assert_eq!(
        merged.supported_parameters,
        vec!["tools".to_string(), "temperature".to_string()]
    );
}

#[test]
fn merge_hardcoded_limits_take_precedence() {
    use everruns_provider::model::ModelLimits;

    let hardcoded = ModelProfile {
        limits: Some(ModelLimits {
            context: 128_000,
            input: None,
            output: 16_384,
            max_media: None,
        }),
        ..base_profile()
    };
    let discovered = ModelProfile {
        limits: Some(ModelLimits {
            context: 200_000,
            input: None,
            output: 64_000,
            max_media: None,
        }),
        ..base_profile()
    };

    let merged = ModelService::merge_profiles(hardcoded, discovered);
    assert_eq!(merged.limits.unwrap().context, 128_000);
}

#[test]
fn merge_preserves_hardcoded_verbosity() {
    use everruns_provider::model::{Verbosity, VerbosityConfig, VerbosityValue};

    let hardcoded = ModelProfile {
        verbosity: Some(VerbosityConfig {
            values: vec![VerbosityValue {
                value: Verbosity::Medium,
                name: "Medium".into(),
            }],
            default: Verbosity::Medium,
        }),
        ..base_profile()
    };

    let merged = ModelService::merge_profiles(hardcoded, base_profile());

    assert_eq!(merged.verbosity.unwrap().default, Verbosity::Medium);
}

#[test]
fn extract_discovered_profile_from_metadata() {
    use crate::storage::models::ModelWithProviderRow;
    use chrono::Utc;

    let profile = base_profile();
    let metadata = serde_json::json!({
        "discovered_profile": profile,
    });

    let row = ModelWithProviderRow {
        id: everruns_provider::typed_id::ModelId::new(),
        org_id: 1,
        provider_id: everruns_provider::typed_id::ProviderId::new(),
        model_id: "test-model".into(),
        display_name: "Test".into(),
        capabilities: serde_json::json!([]),
        is_favorite: false,
        enabled: true,
        source: "discovered".into(),
        last_seen_at: None,
        provider_metadata: Some(metadata),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        provider_name: "TestProvider".into(),
        provider_type: "anthropic".into(),
        provider_api_key_set: true,
        provider_status: "active".into(),
    };

    let extracted = ModelService::extract_discovered_profile(&row);
    assert!(extracted.is_some());
    assert_eq!(extracted.unwrap().name, "Test");
}

#[test]
fn extract_discovered_profile_returns_none_without_metadata() {
    use crate::storage::models::ModelWithProviderRow;
    use chrono::Utc;

    let row = ModelWithProviderRow {
        id: everruns_provider::typed_id::ModelId::new(),
        org_id: 1,
        provider_id: everruns_provider::typed_id::ProviderId::new(),
        model_id: "test-model".into(),
        display_name: "Test".into(),
        capabilities: serde_json::json!([]),
        is_favorite: false,
        enabled: true,
        source: "manual".into(),
        last_seen_at: None,
        provider_metadata: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        provider_name: "TestProvider".into(),
        provider_type: "openai".into(),
        provider_api_key_set: true,
        provider_status: "active".into(),
    };

    let extracted = ModelService::extract_discovered_profile(&row);
    assert!(extracted.is_none());
}
