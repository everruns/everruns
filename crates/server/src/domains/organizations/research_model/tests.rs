use super::*;
use crate::storage::ModelWithProviderRow;
use chrono::Utc;
use everruns_contracts::typed_id::ProviderId;

fn model(provider_type: &str, model_id: &str) -> ModelWithProviderRow {
    ModelWithProviderRow {
        id: ModelId::new(),
        org_id: 1,
        provider_id: ProviderId::new(),
        model_id: model_id.into(),
        display_name: model_id.into(),
        capabilities: serde_json::json!([]),
        is_favorite: false,
        enabled: true,
        source: "predefined".into(),
        last_seen_at: None,
        provider_metadata: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        provider_name: provider_type.into(),
        provider_type: provider_type.into(),
        provider_api_key_set: true,
        provider_status: "active".into(),
    }
}

fn pick(models: &[ModelWithProviderRow]) -> Option<ModelId> {
    automatic_pick(&crate::domains::common::default_driver_registry(), models)
}

#[test]
fn automatic_prefers_anthropic_research_model_over_openai() {
    let astra = model("openai", "gpt-6-astra");
    let fable = model("anthropic", "claude-fable-5-2");
    let models = [model("openai", "gpt-6.1-sol"), astra, fable.clone()];
    assert_eq!(pick(&models), Some(fable.id));
}

#[test]
fn automatic_uses_openai_when_anthropic_is_missing_or_unusable() {
    let astra = model("openai", "gpt-6-astra");
    let mut no_key = model("anthropic", "claude-fable-5-2");
    no_key.provider_api_key_set = false;
    let mut disabled = model("anthropic", "claude-fable-5-2");
    disabled.enabled = false;
    assert_eq!(pick(&[no_key, disabled, astra.clone()]), Some(astra.id));
}

#[test]
fn automatic_falls_back_to_other_providers_then_none() {
    let gemini = model("gemini", "gemini-4-argon");
    assert_eq!(
        pick(&[model("gemini", "gemini-4-flash"), gemini.clone()]),
        Some(gemini.id)
    );
    // Only non-research models: the caller uses the org default.
    assert_eq!(pick(&[model("anthropic", "claude-opus-5-5")]), None);
    assert_eq!(pick(&[]), None);
}

#[test]
fn automatic_skips_personal_providers() {
    assert_eq!(pick(&[model("chatgpt", "gpt-6-astra")]), None);
}

#[test]
fn model_role_round_trips_through_storage_text() {
    let role = AgentModelRole::Research;
    assert_eq!(AgentModelRole::from_db(Some(role.as_str())), Some(role));
    assert_eq!(AgentModelRole::from_db(Some("other")), None);
    assert_eq!(AgentModelRole::from_db(None), None);
    assert_eq!(serde_json::to_value(role).unwrap(), "research");
}
