// Built-in model presets, computed from an org's models and the drivers'
// curated picks (`ProviderRecommendedModels`).
//
// Decision: built-ins are pure functions of (registry, models) so they always
// follow the latest curated picks and the org's current connections, and are
// testable without a database. Only models that can serve turns now count
// (`research_model::usable`).

use super::types::*;
use crate::storage::ModelWithProviderRow;
use everruns_contracts::driver_registry::DriverRegistry;
use everruns_contracts::provider::DriverId;
use everruns_contracts::recommended_models::ProviderRecommendedModels;
use everruns_contracts::typed_id::ModelId;

/// Provider types Automatic tries first, in order (matches the research
/// model's Automatic order).
const PREFERRED_PROVIDERS: [&str; 2] = ["anthropic", "openai"];

/// The curated model name for a job. The Default job uses the chat pick.
fn pick(picks: &ProviderRecommendedModels, job: ModelJob) -> &'static str {
    match job {
        ModelJob::Default | ModelJob::Chat => picks.chat_model,
        ModelJob::Coding => picks.coding_model,
        ModelJob::Research => picks.research_model,
        ModelJob::Small => picks.small_model,
        ModelJob::Decision => picks.decision_model,
    }
}

fn picks_for(registry: &DriverRegistry, provider_type: &str) -> Option<ProviderRecommendedModels> {
    registry.recommended_models(&provider_type.parse::<DriverId>().ok()?)
}

fn usable(model: &ModelWithProviderRow) -> bool {
    crate::domains::organizations::research_model::usable(model)
}

/// Automatic: per job, the first usable model that is its provider's curated
/// pick for that job, preferring Anthropic, then OpenAI, then other providers
/// in listing order.
pub fn automatic(registry: &DriverRegistry, models: &[ModelWithProviderRow]) -> ModelPresetJobs {
    let rank = |model: &ModelWithProviderRow| {
        PREFERRED_PROVIDERS
            .iter()
            .position(|kind| *kind == model.provider_type)
            .unwrap_or(PREFERRED_PROVIDERS.len())
    };
    let mut jobs = ModelPresetJobs::default();
    for job in ModelJob::ALL {
        let chosen = models
            .iter()
            .enumerate()
            .filter(|(_, model)| {
                usable(model)
                    && picks_for(registry, &model.provider_type)
                        .is_some_and(|picks| pick(&picks, job) == model.model_id)
            })
            .min_by_key(|(index, model)| (rank(model), *index))
            .map(|(_, model)| model.id);
        jobs.set(job, chosen);
    }
    jobs
}

/// One provider's preset: its curated pick per job, else the provider's next
/// recommended usable model (weakest first for Small and Decision), flagged
/// in the returned fallback list.
pub fn provider(
    picks: &ProviderRecommendedModels,
    models: &[ModelWithProviderRow],
) -> (ModelPresetJobs, Vec<ModelJob>) {
    let usable_named = |name: &str| -> Option<ModelId> {
        models
            .iter()
            .find(|model| {
                model.provider_type == picks.provider && model.model_id == name && usable(model)
            })
            .map(|model| model.id)
    };
    let mut jobs = ModelPresetJobs::default();
    let mut fallback = Vec::new();
    for job in ModelJob::ALL {
        if let Some(id) = usable_named(pick(picks, job)) {
            jobs.set(job, Some(id));
            continue;
        }
        let cheap_first = matches!(job, ModelJob::Small | ModelJob::Decision);
        let mut names: Vec<&str> = picks.recommended_models.to_vec();
        if cheap_first {
            names.reverse();
        }
        if let Some(id) = names.into_iter().find_map(usable_named) {
            jobs.set(job, Some(id));
            fallback.push(job);
        }
    }
    (jobs, fallback)
}

/// Every built-in preset for the org: Automatic, then one per connected
/// provider type with curated picks and at least one usable model, by name.
pub fn all(registry: &DriverRegistry, models: &[ModelWithProviderRow]) -> Vec<ModelPreset> {
    let mut presets = vec![ModelPreset {
        id: AUTOMATIC_PRESET_ID.to_string(),
        name: "Automatic".to_string(),
        description: Some("The best usable model for each job across your providers.".to_string()),
        kind: ModelPresetKind::Automatic,
        provider_type: None,
        read_only: true,
        jobs: automatic(registry, models),
        fallback_jobs: Vec::new(),
        agent_count: 0,
        created_at: None,
        updated_at: None,
    }];
    let mut seen: Vec<&str> = Vec::new();
    let mut providers = Vec::new();
    for model in models.iter().filter(|model| usable(model)) {
        let kind = model.provider_type.as_str();
        if seen.contains(&kind) {
            continue;
        }
        seen.push(kind);
        let Ok(driver) = kind.parse::<DriverId>();
        let Some(picks) = registry.recommended_models(&driver) else {
            continue;
        };
        let (jobs, fallback_jobs) = provider(&picks, models);
        if jobs == ModelPresetJobs::default() {
            continue;
        }
        let name = registry
            .descriptor(&driver)
            .map(|descriptor| descriptor.display_name.clone())
            .unwrap_or_else(|| kind.to_string());
        providers.push(ModelPreset {
            id: format!("{PROVIDER_PRESET_PREFIX}{kind}"),
            description: Some(format!("{name}'s recommended model for each job.")),
            name,
            kind: ModelPresetKind::Provider,
            provider_type: Some(kind.to_string()),
            read_only: true,
            jobs,
            fallback_jobs,
            agent_count: 0,
            created_at: None,
            updated_at: None,
        });
    }
    providers.sort_by_key(|preset| preset.name.to_lowercase());
    presets.extend(providers);
    presets
}
