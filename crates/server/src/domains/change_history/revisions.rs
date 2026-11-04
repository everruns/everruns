// Revision commands: `everruns history show|diff|restore`.
//
// Decision: show and diff are reads with the same visibility as `history
// list`. Restore is not a rewind: it turns the snapshot into the kind's own
// update command and runs it through `Command::run`, so permission,
// validation, manager-context acknowledgement and the reason apply exactly as
// for a hand-written update, and that update records as `restored`. Restore
// declares itself exempt because the update it runs is the recorded change.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::commands::{EntityChange, HISTORY_READ, readable};
use super::snapshot::{self, FieldDiff};
use crate::domains::common::*;
use crate::storage::entity_changes::{EntityRevisionKey, EntityRevisionRow};

/// One revision of an entity: the entry that made it and the entity as it
/// stood after it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EntityRevision {
    pub revision: i64,
    pub change: EntityChange,
    /// The entity after the change, secrets as markers under `$secrets`.
    /// `null` once the revision is older than the snapshots kept.
    pub snapshot: Option<Value>,
}

async fn revision(
    ctx: &Ctx,
    entity_ref: &str,
    kind: super::EntityKind,
    revision: Option<i64>,
) -> Result<EntityRevisionRow, CommandError> {
    let key = EntityRevisionKey {
        org_id: ctx.org_id(),
        entity_kind: kind.as_str().to_string(),
        entity_ref: entity_ref.to_string(),
        revision,
    };
    ctx.db
        .get_entity_revision(&key)
        .await?
        .ok_or_else(|| match revision {
            Some(revision) => {
                CommandError::not_found_msg(format!("{entity_ref} has no revision {revision}"))
            }
            None => CommandError::not_found_msg(format!("{entity_ref} has no revisions")),
        })
}

fn snapshot_of(row: &EntityRevisionRow) -> Result<&Value, CommandError> {
    row.snapshot.as_ref().ok_or_else(|| {
        CommandError::not_found_msg(format!(
            "The snapshot of revision {} is older than the snapshots kept",
            row.entry.revision.unwrap_or_default()
        ))
    })
}

// ============================================================================
// ShowEntityRevision
// ============================================================================

/// Show an entity as it stood at one revision.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct ShowEntityRevision {
    /// The entity's public id.
    pub entity_ref: String,
    /// Entity kind, for ids without a prefix.
    pub kind: Option<String>,
    /// Revision number; the latest when omitted.
    pub revision: Option<i64>,
}

#[command(
    name = "show_entity_revision",
    category = "history",
    description = "Show an entity as it stood at one revision of its history (the latest by default). Secrets appear only as markers saying whether they were set.",
    method = "GET",
    path = "/v1/history/{entity_ref}/revisions/{revision}",
    policy = HISTORY_READ,
    positional = "entity_ref",
    cli = CliRoute::new(&["history"], "show").with_args(&[CliArg::new("entity_ref").at(1)]).with_examples(&[CliExample::new("See an agent's configuration before last week's change", "everruns history show agent_01h9 --revision 4",)]),
)]
impl Command for ShowEntityRevision {
    type Output = EntityRevision;

    fn output_shape() -> &'static str {
        "unknown"
    }

    async fn execute(self, ctx: &Ctx) -> Result<EntityRevision, CommandError> {
        let (entity_ref, kind) = readable(ctx, &self.entity_ref, self.kind.as_deref()).await?;
        let row = revision(ctx, &entity_ref, kind, self.revision).await?;
        Ok(EntityRevision {
            revision: row.entry.revision.unwrap_or_default(),
            snapshot: row.snapshot,
            change: row.entry.into(),
        })
    }
}

// ============================================================================
// DiffEntityRevisions
// ============================================================================

/// Compare two revisions of an entity field by field.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct DiffEntityRevisions {
    /// The entity's public id.
    pub entity_ref: String,
    /// Entity kind, for ids without a prefix.
    pub kind: Option<String>,
    /// The older revision.
    pub from: i64,
    /// The newer revision; the latest when omitted.
    pub to: Option<i64>,
}

#[command(
    name = "diff_entity_revisions",
    category = "history",
    description = "Compare two revisions of an entity field by field (to the latest by default). Secrets compare only as set or changed.",
    method = "GET",
    path = "/v1/history/{entity_ref}/diff",
    policy = HISTORY_READ,
    positional = "entity_ref",
    cli = CliRoute::new(&["history"], "diff").with_args(&[CliArg::new("entity_ref").at(1)]).with_examples(&[CliExample::new("See what changed in an agent since revision 4", "everruns history diff agent_01h9 --from 4",)]),
)]
impl Command for DiffEntityRevisions {
    type Output = Vec<FieldDiff>;

    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<FieldDiff>())
    }

    fn output_shape() -> &'static str {
        "array"
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<FieldDiff>, CommandError> {
        let (entity_ref, kind) = readable(ctx, &self.entity_ref, self.kind.as_deref()).await?;
        let from = revision(ctx, &entity_ref, kind, Some(self.from)).await?;
        let to = revision(ctx, &entity_ref, kind, self.to).await?;
        Ok(snapshot::diff_revisions(
            kind,
            snapshot_of(&from)?,
            snapshot_of(&to)?,
        ))
    }
}

// ============================================================================
// RestoreEntityRevision
// ============================================================================

/// What a restore did.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RestoreResult {
    /// The committed inverse's revision, for correlating subsequent reads.
    pub revision: Option<i64>,
    /// The revision brought back.
    pub restored_revision: i64,
    /// The entity after the restore, as its update command returned it.
    pub entity: Value,
    /// Secrets kept at their current value that differ from the revision.
    pub warnings: Vec<String>,
}

/// Make an entity look like it did at a revision, as a new change.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct RestoreEntityRevision {
    /// The entity's public id.
    pub entity_ref: String,
    /// Entity kind, for ids without a prefix.
    pub kind: Option<String>,
    /// The revision to bring back.
    pub revision: i64,
    /// Refuse if the resource changed after this revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<i64>,
    /// Restore only these top-level fields; omit to restore the full definition.
    /// Requires expected_revision. Secrets and fields the update cannot restore are refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
}

#[command(
    name = "restore_entity_revision",
    category = "history",
    description = "Make an entity look like it did at a revision. Runs as a new change through the entity's own update command, recorded as `restored`; secrets keep their current value.",
    method = "POST",
    path = "/v1/history/{entity_ref}/restore",
    policy = HISTORY_READ,
    positional = "entity_ref",
    cli = CliRoute::new(&["history"], "restore").with_args(&[CliArg::new("entity_ref").at(1)]).with_examples(&[CliExample::new("Put an agent back the way it was before a bad edit", "everruns history restore agent_01h9 --revision 4 --reason 'Revert the prompt change'",)]),
)]
impl Command for RestoreEntityRevision {
    type Output = RestoreResult;

    fn output_shape() -> &'static str {
        "unknown"
    }

    fn change() -> super::Change {
        super::Change::Exempt("runs the entity's update command, which records the restore")
    }

    async fn execute(self, ctx: &Ctx) -> Result<RestoreResult, CommandError> {
        let (entity_ref, kind) = readable(ctx, &self.entity_ref, self.kind.as_deref()).await?;
        if !(kind == super::EntityKind::VirtualUser
            && crate::domains::virtual_users::self_profile::owns(ctx, &entity_ref).await)
        {
            kind.manage_policy()
                .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
                .map_err(|e| CommandError::forbidden(e.message))?;
        }
        if self.fields.is_some() && self.expected_revision.is_none() {
            return Err(CommandError::bad_request(
                "Field restore requires expected_revision",
            ));
        }
        super::mutation_guard::run(
            ctx,
            super::mutation_guard::Key::new(ctx, kind, &entity_ref),
            true,
            self.restore(ctx),
        )
        .await
    }
}

impl RestoreEntityRevision {
    async fn restore(self, ctx: &Ctx) -> Result<RestoreResult, CommandError> {
        let (entity_ref, kind) = readable(ctx, &self.entity_ref, self.kind.as_deref()).await?;
        if !(kind == super::EntityKind::VirtualUser
            && crate::domains::virtual_users::self_profile::owns(ctx, &entity_ref).await)
        {
            kind.manage_policy()
                .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
                .map_err(|e| CommandError::forbidden(e.message))?;
        }
        if self.expected_revision.is_some() || self.fields.is_some() {
            if self.expected_revision.is_none() {
                return Err(CommandError::bad_request(
                    "Guarded field restore requires expected_revision",
                ));
            }
            if kind == super::EntityKind::Agent {
                ctx.db
                    .lock_agent_for_change(ctx.org_id(), &entity_ref)
                    .await?;
            }
            let latest = ctx
                .db
                .latest_entity_revision(ctx.org_id(), kind.as_str(), &entity_ref)
                .await?;
            if latest != self.expected_revision {
                return Err(CommandError::conflict("This resource changed since it was saved. Review the latest changes before undoing.").with_code("entity_revision_changed"));
            }
        }
        let row = revision(ctx, &entity_ref, kind, Some(self.revision)).await?;
        let restored = snapshot_of(&row)?;
        let Some(current) = snapshot::read(ctx, kind, &entity_ref).await.or_else(|| {
            // Kinds read through a parent: their latest snapshot is current.
            Some(restored.clone()).filter(|_| kind.lookup().is_none())
        }) else {
            return Err(CommandError::not_found_msg(format!(
                "{entity_ref} no longer exists; restoring a deleted {} is not supported",
                kind.as_str()
            )));
        };
        if self.fields.is_some()
            && (kind == super::EntityKind::Schedule
                || (kind == super::EntityKind::AgentTrigger
                    && current["trigger_type"] == "schedule"))
        {
            let pool = ctx
                .workflow_store
                .as_ref()
                .and_then(|store| store.schedule_transaction_pool());
            if !pool.as_ref().is_some_and(|pool| {
                everruns_durable::schedule_transaction::same_pool(pool, ctx.db.pool())
            }) {
                return Err(CommandError::conflict(
                    "This schedule store cannot participate in atomic Undo. Review and edit the schedule directly.",
                ));
            }
        }
        if self.fields.is_some()
            && kind == super::EntityKind::AgentTrigger
            && current["trigger_type"] == "mcp_event"
        {
            return Err(CommandError::conflict(
                "MCP event subscriptions must be changed directly so remote subscription effects can be reviewed.",
            ));
        }
        if let Some(expected) = self.expected_revision {
            let latest = revision(ctx, &entity_ref, kind, Some(expected)).await?;
            let latest = snapshot_of(&latest)?;
            let captured = snapshot::capture(
                ctx,
                kind,
                super::ChangeAction::Updated,
                &entity_ref,
                &Value::Null,
            )
            .await;
            if !captured
                .as_ref()
                .is_some_and(|(now, _)| snapshot::matches_current(kind, latest, now))
            {
                return Err(CommandError::conflict("This resource changed outside its recorded revision. Review the current configuration before undoing.").with_code("entity_revision_changed"));
            }
        }
        let (command, mut params) = snapshot::restore_call(kind, &entity_ref, restored, &current)?;
        let id_param = snapshot::restore_command(kind).map_or("id", |(_, id)| id);
        let locators = snapshot::restore_locators(kind);
        if let Some(fields) = &self.fields {
            let values = params
                .as_object_mut()
                .expect("restore params are an object");
            if fields.is_empty()
                || fields.iter().any(|field| {
                    field == id_param
                        || locators.contains(&field.as_str())
                        || !values.contains_key(field)
                })
            {
                return Err(CommandError::bad_request(
                    "Choose non-secret fields that this resource's update can restore",
                ));
            }
            values.retain(|field, _| {
                field == id_param || locators.contains(&field.as_str()) || fields.contains(field)
            });
            snapshot::preserve_replacement_fields(
                kind,
                &entity_ref,
                &mut params,
                &current,
                fields,
            )?;
        }
        let command = if kind == super::EntityKind::VirtualUser
            && kind
                .manage_policy()
                .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
                .is_err()
        {
            if params.get("status").is_some() {
                return Err(CommandError::forbidden(
                    "Self-service cannot restore lifecycle. Choose profile fields only.",
                ));
            }
            "update_own_virtual_user_profile"
        } else {
            command
        };
        let current_secrets = snapshot::secret_markers(ctx, kind, &entity_ref, &current).await;
        let warnings = snapshot::kept_warnings(restored, &current, &current_secrets, self.revision);

        let mut intent = super::effective_intent(ctx);
        intent.restoring = Some(self.revision);
        for warning in &warnings {
            intent.notices.push(warning.clone());
        }
        let restore_ctx = ctx.clone().with_change_intent(intent);
        let dispatch_params = if kind == super::EntityKind::AgentChannel {
            crate::domains::agent_channels::history::prepare_restore(&restore_ctx, &params).await?
        } else {
            params.clone()
        };
        let entity = dispatch(command, dispatch_params, &restore_ctx).await?;
        let mut entity: Value = serde_json::from_str(&entity).unwrap_or(Value::Null);
        if kind == super::EntityKind::Skill {
            entity = snapshot::read(&restore_ctx, kind, &entity_ref)
                .await
                .ok_or_else(|| CommandError::not_found("Skill"))?;
        }
        let mut warnings = warnings;
        let unrestored: Vec<_> = snapshot::unrestored(&params, &entity, id_param)
            .into_iter()
            .filter(|field| {
                self.fields
                    .as_ref()
                    .is_none_or(|fields| fields.contains(field))
            })
            .collect();
        if self.fields.is_some() && !unrestored.is_empty() {
            // Guarded field undo is all-or-nothing. The outer transaction also
            // rolls back the nested update's history when an API cannot clear a field.
            return Err(CommandError::conflict(format!(
                "These fields could not be restored: {}. No undo was saved.",
                unrestored.join(", ")
            ))
            .with_code("field_restore_failed"));
        }
        for field in unrestored {
            let warning = format!(
                "{field} could not be set back to its revision {} value by `{command}`; \
                 change it directly",
                self.revision
            );
            restore_ctx
                .change_intent
                .iter()
                .for_each(|i| i.notices.push(warning.clone()));
            warnings.push(warning);
        }
        Ok(RestoreResult {
            revision: ctx
                .db
                .latest_entity_revision(ctx.org_id(), kind.as_str(), &entity_ref)
                .await?,
            restored_revision: self.revision,
            entity,
            warnings,
        })
    }
}
