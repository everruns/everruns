//! System-managed observers share the scoring queue and org visibility of
//! user observers. Consent is rechecked independently of queued configuration.
mod evidence;
mod listener;
pub mod policy;
mod worker;
pub use listener::enqueue_event;
pub use policy::QualityMonitoringPolicy;
pub use worker::process_score;
#[cfg(test)]
mod tests;

use super::record::{CompactionCheck, ObserverScope, ObserverScorerConfig, ScorerMethod};
use crate::storage::StorageBackend;

pub const EXECUTION_KEY: &str = "compaction_execution_v1";
pub const CONTINUITY_KEY: &str = "compaction_continuity_v1";
pub const EVENT_TYPES: &[&str] = &[
    "context.compacted",
    "context.compaction.failed",
    "context.compaction.skipped",
    "turn.completed",
    "turn.failed",
];

pub async fn policy(db: &StorageBackend, org_id: i64) -> anyhow::Result<QualityMonitoringPolicy> {
    Ok(db
        .org_quality_monitoring(org_id)
        .await?
        .map(serde_json::from_value)
        .transpose()?
        .unwrap_or_default())
}

pub async fn validate_policy(
    db: &StorageBackend,
    org_id: i64,
    policy: &QualityMonitoringPolicy,
) -> Result<(), String> {
    policy.validate()?;
    if let Some(model_id) = policy.model_id.filter(|_| policy.enabled) {
        let model = db
            .get_model_with_provider(org_id, model_id.uuid())
            .await
            .map_err(|_| "Unable to validate monitoring model")?;
        let model = model
            .filter(|model| model.enabled)
            .ok_or("Monitoring model must be enabled and belong to this organization")?;
        crate::domains::models::catalog::require_chat(model.provider_metadata.as_ref())
            .map_err(|_| "Monitoring model must be a chat model")?;
        if model.provider_type == "chatgpt" {
            return Err(
                "Monitoring requires an organization provider, not a personal provider".into(),
            );
        }
        if model.provider_status != "active"
            || (!model.provider_api_key_set && model.provider_type != "llmsim")
        {
            return Err("Monitoring model must have an available provider".into());
        }
    }
    Ok(())
}

pub async fn configure(
    db: &StorageBackend,
    org_id: i64,
    policy: &QualityMonitoringPolicy,
) -> anyhow::Result<()> {
    let scorers = [
        ObserverScorerConfig {
            key: EXECUTION_KEY.into(),
            scope: ObserverScope::Compaction,
            tools: None,
            method: ScorerMethod::SystemCompaction {
                check: CompactionCheck::Execution,
            },
        },
        ObserverScorerConfig {
            key: CONTINUITY_KEY.into(),
            scope: ObserverScope::Compaction,
            tools: None,
            method: ScorerMethod::SystemCompaction {
                check: CompactionCheck::Continuity,
            },
        },
    ];
    db.set_org_quality_monitoring(
        org_id,
        &serde_json::to_value(policy)?,
        &serde_json::to_value(scorers)?,
        policy.enabled,
    )
    .await?;
    crate::domains::data_plane::event_tap::resync(db, org_id).await;
    Ok(())
}
