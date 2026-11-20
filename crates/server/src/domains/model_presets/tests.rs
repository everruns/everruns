use super::builtin;
use super::types::*;
use crate::storage::ModelWithProviderRow;
use chrono::Utc;
use everruns_contracts::driver_registry::DriverRegistry;
use everruns_contracts::provider::DriverId;
use everruns_contracts::recommended_models::ProviderRecommendedModels;
use everruns_contracts::typed_id::{ModelId, ProviderId};

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

fn registry() -> std::sync::Arc<DriverRegistry> {
    crate::domains::common::default_driver_registry()
}

fn picks(provider: &str) -> ProviderRecommendedModels {
    registry()
        .recommended_models(&provider.parse::<DriverId>().unwrap())
        .expect("driver declares curated picks")
}

/// Every model a provider's curated picks name, once each.
fn all_picked(provider: &str) -> Vec<ModelWithProviderRow> {
    let p = picks(provider);
    let mut names = vec![
        p.chat_model,
        p.coding_model,
        p.research_model,
        p.small_model,
        p.decision_model,
    ];
    names.dedup();
    let mut seen = Vec::new();
    names
        .into_iter()
        .filter(|name| {
            let fresh = !seen.contains(name);
            seen.push(*name);
            fresh
        })
        .map(|name| model(provider, name))
        .collect()
}

fn id_of(models: &[ModelWithProviderRow], name: &str) -> Option<ModelId> {
    models.iter().find(|m| m.model_id == name).map(|m| m.id)
}

#[test]
fn provider_preset_uses_curated_pick_for_each_job() {
    let models = all_picked("gemini");
    let p = picks("gemini");
    let (jobs, fallback) = builtin::provider(&p, &models);
    assert!(fallback.is_empty());
    assert_eq!(jobs.chat_model_id, id_of(&models, p.chat_model));
    assert_eq!(jobs.default_model_id, id_of(&models, p.chat_model));
    assert_eq!(jobs.coding_model_id, id_of(&models, p.coding_model));
    assert_eq!(jobs.research_model_id, id_of(&models, p.research_model));
    assert_eq!(jobs.small_model_id, id_of(&models, p.small_model));
    assert_eq!(jobs.decision_model_id, id_of(&models, p.decision_model));
}

#[test]
fn provider_preset_falls_back_and_flags_missing_picks() {
    let p = picks("openai");
    // Only the research pick is connected: every other job falls back to it.
    let only = model("openai", p.research_model);
    let models = vec![only.clone()];
    let (jobs, fallback) = builtin::provider(&p, &models);
    assert_eq!(jobs.research_model_id, Some(only.id));
    for job in ModelJob::ALL {
        let pick_is_research = match job {
            ModelJob::Default | ModelJob::Chat => p.chat_model == p.research_model,
            ModelJob::Coding => p.coding_model == p.research_model,
            ModelJob::Research => true,
            ModelJob::Small => p.small_model == p.research_model,
            ModelJob::Decision => p.decision_model == p.research_model,
        };
        if pick_is_research {
            assert!(!fallback.contains(&job), "{job:?}");
        } else if p.recommended_models.contains(&p.research_model) {
            assert!(fallback.contains(&job), "{job:?}");
            assert_eq!(jobs.get(job), Some(only.id));
        }
    }
}

#[test]
fn provider_preset_ignores_unusable_and_other_providers_models() {
    let p = picks("anthropic");
    let mut disabled = model("anthropic", p.chat_model);
    disabled.enabled = false;
    let other = model("openai", p.chat_model);
    let (jobs, _) = builtin::provider(&p, &[disabled, other]);
    assert_eq!(jobs, ModelPresetJobs::default());
}

#[test]
fn automatic_prefers_anthropic_then_openai_per_job() {
    let mut models = all_picked("openai");
    models.extend(all_picked("anthropic"));
    let jobs = builtin::automatic(&registry(), &models);
    let a = picks("anthropic");
    assert_eq!(
        jobs.coding_model_id,
        models
            .iter()
            .find(|m| m.provider_type == "anthropic" && m.model_id == a.coding_model)
            .map(|m| m.id)
    );
    // Without Anthropic, OpenAI's picks are used.
    let openai = all_picked("openai");
    let o = picks("openai");
    let jobs = builtin::automatic(&registry(), &openai);
    assert_eq!(jobs.small_model_id, id_of(&openai, o.small_model));
}

#[test]
fn builtins_list_automatic_then_connected_providers_by_name() {
    let mut models = all_picked("openai");
    models.extend(all_picked("gemini"));
    let mut personal = model("chatgpt", "gpt-6.2-sol");
    personal.provider_api_key_set = true;
    models.push(personal);
    let presets = builtin::all(&registry(), &models);
    let ids: Vec<&str> = presets.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids[0], AUTOMATIC_PRESET_ID);
    assert!(ids.contains(&"provider:openai"));
    assert!(ids.contains(&"provider:gemini"));
    assert!(!ids.iter().any(|id| id.contains("chatgpt")));
    assert!(presets.iter().all(|p| p.read_only));
    let names: Vec<String> = presets[1..].iter().map(|p| p.name.to_lowercase()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
}

#[test]
fn builtins_without_models_is_only_automatic_with_no_jobs() {
    let presets = builtin::all(&registry(), &[]);
    assert_eq!(presets.len(), 1);
    assert_eq!(presets[0].jobs, ModelPresetJobs::default());
}

#[test]
fn update_request_keeps_omitted_and_clears_null_jobs() {
    let a = ModelId::new();
    let b = ModelId::new();
    let mut jobs = ModelPresetJobs {
        chat_model_id: Some(a),
        small_model_id: Some(a),
        ..Default::default()
    };
    let req: UpdateModelPresetRequest = serde_json::from_value(serde_json::json!({
        "small_model_id": null,
        "coding_model_id": b.to_string(),
    }))
    .unwrap();
    req.apply_jobs(&mut jobs);
    assert_eq!(jobs.chat_model_id, Some(a));
    assert_eq!(jobs.small_model_id, None);
    assert_eq!(jobs.coding_model_id, Some(b));
}
