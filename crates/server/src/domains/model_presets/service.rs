// Model preset reads and resolution shared by commands and the turn model
// resolver (knowledge/foundations/model-presets.md).

use super::builtin;
use super::types::*;
use crate::storage::{ModelPresetRow, StorageBackend};
use anyhow::Result;
use everruns_contracts::typed_id::ModelId;
use std::collections::HashMap;

fn custom(row: &ModelPresetRow) -> ModelPreset {
    ModelPreset {
        id: row.key.clone(),
        name: row.name.clone(),
        description: row.description.clone(),
        kind: ModelPresetKind::Custom,
        provider_type: None,
        read_only: false,
        jobs: ModelPresetJobs::from(row),
        fallback_jobs: Vec::new(),
        agent_count: 0,
        created_at: Some(row.created_at),
        updated_at: Some(row.updated_at),
    }
}

async fn builtins(db: &StorageBackend, org_id: i64) -> Result<Vec<ModelPreset>> {
    let models = db.list_all_models(org_id).await?;
    Ok(builtin::all(
        &crate::domains::common::default_driver_registry(),
        &models,
    ))
}

/// Every preset of the org: built-ins first, then custom presets by name,
/// each with how many live agents use it.
pub async fn list(db: &StorageBackend, org_id: i64) -> Result<Vec<ModelPreset>> {
    let mut presets = builtins(db, org_id).await?;
    presets.extend(db.list_model_presets(org_id).await?.iter().map(custom));
    let counts: HashMap<String, i64> = db
        .count_agents_by_model_preset(org_id)
        .await?
        .into_iter()
        .collect();
    for preset in &mut presets {
        preset.agent_count = counts.get(&preset.id).copied().unwrap_or(0);
    }
    Ok(presets)
}

/// One preset by id, without the agent count.
async fn find(db: &StorageBackend, org_id: i64, id: &str) -> Result<Option<ModelPreset>> {
    if id == AUTOMATIC_PRESET_ID || id.starts_with(PROVIDER_PRESET_PREFIX) {
        return Ok(builtins(db, org_id)
            .await?
            .into_iter()
            .find(|preset| preset.id == id));
    }
    Ok(db.get_model_preset(org_id, id).await?.as_ref().map(custom))
}

/// One preset by id, with the agent count.
pub async fn get(db: &StorageBackend, org_id: i64, id: &str) -> Result<Option<ModelPreset>> {
    let Some(mut preset) = find(db, org_id, id).await? else {
        return Ok(None);
    };
    preset.agent_count = db
        .count_agents_by_model_preset(org_id)
        .await?
        .into_iter()
        .find_map(|(key, count)| (key == preset.id).then_some(count))
        .unwrap_or(0);
    Ok(Some(preset))
}

/// The model a preset assigns to a job. `None` when the preset no longer
/// exists or leaves the job to the organization default, so the caller
/// continues down its fallback chain.
pub async fn job_model(
    db: &StorageBackend,
    org_id: i64,
    preset_id: &str,
    job: ModelJob,
) -> Result<Option<ModelId>> {
    Ok(find(db, org_id, preset_id)
        .await?
        .and_then(|preset| preset.jobs.get(job)))
}

/// A URL- and CLI-friendly key for a new custom preset, unique in the org and
/// never a built-in id.
pub fn key_from_name(name: &str, taken: &[String]) -> String {
    let mut base = String::new();
    for ch in name.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            base.push(ch);
        } else if !base.is_empty() && !base.ends_with('-') {
            base.push('-');
        }
    }
    let mut base = base.trim_end_matches('-').to_string();
    base.truncate(56);
    let base = base.trim_end_matches('-').to_string();
    let base = if base.is_empty() {
        "preset".to_string()
    } else {
        base
    };
    let free = |key: &str| key != AUTOMATIC_PRESET_ID && !taken.iter().any(|t| t == key);
    if free(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|key| free(key))
        .expect("an unused suffix exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_from_name_slugs_and_avoids_collisions() {
        assert_eq!(key_from_name("Cheap council", &[]), "cheap-council");
        assert_eq!(key_from_name("  OpenAI / Gemini!! ", &[]), "openai-gemini");
        assert_eq!(key_from_name("***", &[]), "preset");
        assert_eq!(key_from_name("Automatic", &[]), "automatic-2");
        assert_eq!(
            key_from_name("Fast", &["fast".into(), "fast-2".into()]),
            "fast-3"
        );
        assert!(key_from_name(&"x".repeat(200), &[]).len() <= 63);
    }
}
