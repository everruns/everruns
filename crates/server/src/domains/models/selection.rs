//! Atomic model enablement reviewed against the values shown by the picker.
use super::{LLM_MODEL_MANAGE, LLM_MODEL_VIEW};
use crate::domains::change_history::{EntityKind, mutation_guard};
use crate::domains::common::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ModelSelectionChange {
    pub id: String,
    pub provider_id: String,
    pub expected_enabled: bool,
    pub enabled: bool,
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ApplyModelSelection {
    pub changes: Vec<ModelSelectionChange>,
}
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct ModelSelectionReceipt {
    pub id: String,
    pub from_revision: i64,
    pub revision: i64,
    pub before: bool,
    pub after: bool,
    pub label: String,
}
fn validate_ids(ids: &[String]) -> Result<(), CommandError> {
    if ids.len() > 2000 {
        return Err(CommandError::bad_request(
            "Choose at most 2000 models at a time",
        ));
    }
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    if unique.len() != ids.len() {
        return Err(CommandError::bad_request("Duplicate model selection"));
    }
    for id in ids {
        super::queries::parse_model_id(id)?;
    }
    Ok(())
}
#[command(name = "apply_model_selection", category = "models", description = "Atomically apply reviewed enablement changes. Rejects a model whose provider or enabled state changed while the picker was open.", method = "POST", path = "/v1/models/selection", policy = LLM_MODEL_MANAGE, http = plain, cli = CliRoute::new(&["models", "selection"], "apply").with_examples(&[CliExample::new("Enable a reviewed model only if its provider and current selection still match", "everruns models selection apply --changes '[{\"id\":\"model_01h9\",\"provider_id\":\"provider_01h9\",\"expected_enabled\":false,\"enabled\":true}]' --reason 'Make the reviewed model selectable'")]))]
impl Command for ApplyModelSelection {
    type Output = Vec<ModelSelectionReceipt>;
    fn transactional() -> bool {
        true
    }
    async fn execute(mut self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        LLM_MODEL_VIEW
            .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
            .map_err(|error| CommandError::forbidden(error.message))?;
        validate_ids(
            &self
                .changes
                .iter()
                .map(|item| item.id.clone())
                .collect::<Vec<_>>(),
        )?;
        self.changes.sort_by(|a, b| a.id.cmp(&b.id));
        let mut receipts = Vec::new();
        for change in self.changes {
            let key = mutation_guard::Key::new(ctx, EntityKind::Model, &change.id);
            let receipt = mutation_guard::run(ctx, key, true, async {
                let current = super::GetModel { id: change.id.clone() }.run(ctx).await?;
                if current.provider_id.to_string() != change.provider_id || current.enabled != change.expected_enabled {
                    return Err(CommandError::conflict("A selected model changed. Your selection is kept; review the current catalog before saving."));
                }
                let from = ctx.db.latest_entity_revision(ctx.org_id(), "model", &change.id).await?.unwrap_or(0);
                let command: super::UpdateModel = serde_json::from_value(serde_json::json!({"id": change.id, "enabled": change.enabled})).map_err(|error| CommandError::internal(error.into()))?;
                let updated = command.run(ctx).await?;
                let revision = ctx.db.latest_entity_revision(ctx.org_id(), "model", &change.id).await?.ok_or_else(|| CommandError::conflict("Could not record model review"))?;
                Ok(ModelSelectionReceipt { id: change.id, from_revision: from.max(1), revision, before: current.enabled, after: updated.enabled, label: updated.display_name })
            }).await?;
            if receipt.before != receipt.after {
                receipts.push(receipt);
            }
        }
        Ok(receipts)
    }
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UndoModelSelectionItem {
    pub id: String,
    pub from_revision: i64,
    pub expected_revision: i64,
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UndoModelSelection {
    pub changes: Vec<UndoModelSelectionItem>,
}
#[command(name = "undo_model_selection", category = "models", description = "Atomically undo selected reviewed model enablement changes. Every model must still have its expected revision.", method = "POST", path = "/v1/models/selection/undo", policy = LLM_MODEL_MANAGE, http = plain, cli = CliRoute::new(&["models", "selection"], "undo").with_examples(&[CliExample::new("Undo reviewed enablement only while the saved revision is still current", "everruns models selection undo --changes '[{\"id\":\"model_01h9\",\"from_revision\":1,\"expected_revision\":2}]' --reason 'Restore the reviewed selection'")]))]
impl Command for UndoModelSelection {
    type Output = Vec<ModelSelectionReceipt>;
    fn transactional() -> bool {
        true
    }
    async fn execute(mut self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        validate_ids(
            &self
                .changes
                .iter()
                .map(|item| item.id.clone())
                .collect::<Vec<_>>(),
        )?;
        self.changes.sort_by(|a, b| a.id.cmp(&b.id));
        let mut receipts = Vec::new();
        for change in self.changes {
            let key = mutation_guard::Key::new(ctx, EntityKind::Model, &change.id);
            receipts.push(mutation_guard::run(ctx, key, true, async {
                let current = super::GetModel { id: change.id.clone() }.run(ctx).await?;
                let restore: crate::domains::change_history::revisions::RestoreEntityRevision = serde_json::from_value(serde_json::json!({"entity_ref": change.id,"kind":"model","revision":change.from_revision,"expected_revision":change.expected_revision,"fields":["enabled"]})).map_err(|error| CommandError::internal(error.into()))?;
                let result = restore.run(ctx).await?;
                let updated = super::GetModel { id: change.id.clone() }.run(ctx).await?;
                Ok(ModelSelectionReceipt { id: change.id, from_revision: change.expected_revision, revision: result.revision.ok_or_else(|| CommandError::conflict("Could not verify undo"))?, before: current.enabled, after: updated.enabled, label: updated.display_name })
            }).await?);
        }
        Ok(receipts)
    }
}
