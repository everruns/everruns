// Change reasons and entity history.
//
// Every mutating command declares what it changes (`registry`). When one
// succeeds, `Command::run` records an `entity_changes` row: which entity, what
// happened, which fields the caller sent, who did it and through which surface
// and session, and the caller's reason (`intent`). `history list` reads it
// back (`commands`).
//
// Each entity of a managed kind also has manager context (`context`): one
// markdown document its managers keep about it. A change may say which
// revision of it the caller read; a stale one is refused before the command
// runs, and an unacknowledged one leaves a notice.
//
// Design: knowledge/execution/change-reasons-and-manager-context.md.

#[cfg(test)]
mod command_tests;
pub mod commands;
pub mod context;
#[cfg(test)]
mod context_tests;
pub mod intent;
pub mod registry;
pub mod rest;
pub mod revisions;
#[cfg(test)]
mod revisions_tests;
pub mod snapshot;

use serde_json::Value;

use crate::domains::common::{CommandError, CommandMeta, Ctx};
use crate::storage::entity_changes::NewEntityChange;
use crate::storage::manager_context::ManagerContextKey;
pub use intent::{ChangeIntent, ChangeSurface, http_change_intent_layer};
pub use registry::{Change, ChangeAction, EntityKind, SubjectId};

/// Who made a change, as recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorKind {
    User,
    ApiKey,
    AgentSession,
    System,
}

impl ActorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::ApiKey => "api_key",
            Self::AgentSession => "agent_session",
            Self::System => "system",
        }
    }

    fn of(ctx: &Ctx, intent: &ChangeIntent) -> Self {
        if intent.via_session_id.is_some() || ctx.acting_for_session.is_some() {
            Self::AgentSession
        } else if ctx.caller.user_id.is_some() {
            Self::User
        } else if ctx.caller.is_internal {
            Self::System
        } else {
            Self::ApiKey
        }
    }
}

/// Error code: an agent changed an entity without saying why.
pub const REASON_REQUIRED: &str = "reason_required";

/// Agents must say why they change something; people may. Decision: while the
/// org has not turned on `agent_change_reasons_required`, the change goes
/// through with a warning, so agents that predate the rule keep working while
/// the warning teaches them; with it on, the change is refused before it runs
/// and the error says how to retry.
fn missing_agent_reason(
    ctx: &Ctx,
    intent: &ChangeIntent,
    kind: EntityKind,
) -> Result<(), CommandError> {
    let message = format!(
        "Changes made by agents need a reason. Retry with --reason saying what the user \
         asked for, so the {} history records why it changed.",
        kind.as_str()
    );
    if ctx
        .feature_flags
        .is_enabled("agent_change_reasons_required")
    {
        return Err(CommandError::bad_request(message)
            .with_code(REASON_REQUIRED)
            .with_action(crate::common_dto::AllowedAction::new("retry").with_hint(
                "Retry the same command with --reason \"<what the user asked for>\".",
            )));
    }
    metrics::counter!(
        crate::metrics_names::ENTITY_CHANGES_WITHOUT_REASON,
        "entity_kind" => kind.as_str(),
    )
    .increment(1);
    intent.notices.push(message);
    Ok(())
}

/// The intent a command runs under: what its adapter set on `Ctx`, completed
/// by what the HTTP layer captured for the request.
pub fn effective_intent(ctx: &Ctx) -> ChangeIntent {
    let http = intent::current_http_intent();
    match &ctx.change_intent {
        Some(own) => own.clone().or(http.as_ref()),
        None => http.unwrap_or_else(|| ChangeIntent::on(ChangeSurface::Internal)),
    }
}

/// The intent of changes an agent makes from inside `session`: the session and
/// its agent are recorded as the way each change came in. The agent lookup is
/// best effort; a change is never refused because it failed.
pub async fn agent_session_intent(
    db: &crate::storage::StorageBackend,
    org_id: i64,
    session: everruns_contracts::typed_id::SessionId,
    surface: ChangeSurface,
) -> ChangeIntent {
    let via_agent_id = match db.get_session(org_id, session).await {
        Ok(Some(row)) => row.agent_id.map(|agent| agent.to_string()),
        _ => None,
    };
    ChangeIntent {
        surface: Some(surface),
        via_session_id: Some(session.uuid()),
        via_agent_id,
        ..ChangeIntent::default()
    }
}

/// `ctx` for commands MCP `execute` runs (and `/v1/commands`, which overrides).
pub fn on_surface(ctx: Ctx) -> Ctx {
    ctx.with_change_intent(ChangeIntent::on(ChangeSurface::Mcp))
}

/// `ctx` for commands the Platform capability runs inside `session`.
pub async fn on_platform(ctx: Ctx, session: everruns_contracts::typed_id::SessionId) -> Ctx {
    let intent = agent_session_intent(&ctx.db, ctx.org_id(), session, ChangeSurface::Platform);
    let intent = intent.await;
    ctx.with_change_intent(intent)
}

/// Everything about a pending change that is known before the command runs.
pub struct PendingChange {
    kind: EntityKind,
    action: ChangeAction,
    id: SubjectId,
    params: Value,
    intent: ChangeIntent,
    reason: Option<String>,
}

impl PendingChange {
    /// Prepare to record what `command` changes, before it runs: its params
    /// are captured before `execute` consumes it, and the manager context
    /// revision the caller acknowledged is checked. The params are read before
    /// the returned future, so a command need not be `Sync`.
    pub fn of<'a, C: crate::domains::common::Command>(
        command: &C,
        ctx: &'a Ctx,
    ) -> impl std::future::Future<Output = Result<Option<Self>, CommandError>> + Send + 'a {
        let change = C::change();
        let params = matches!(change, Change::Subject { .. })
            .then(|| serde_json::to_value(command).unwrap_or(Value::Null));
        async move {
            let Some(params) = params else {
                return Ok(None);
            };
            let Some(pending) = Self::prepare(change, params, ctx)? else {
                return Ok(None);
            };
            pending.check_context(ctx).await?;
            Ok(Some(pending))
        }
    }

    /// The entity an existing-entity change names, when its kind has manager
    /// context. Creates have no context yet.
    fn context_key(&self, ctx: &Ctx) -> Option<ManagerContextKey> {
        if !self.kind.has_manager_context() || self.action == ChangeAction::Created {
            return None;
        }
        let entity_ref = match self.id {
            SubjectId::Param(field) => subject_ref(&self.params, field)?,
            // Declared by the output's id (most updates echo the entity), so
            // before the run, find the param holding a ref of this kind.
            SubjectId::Output(_) => self.params.as_object()?.values().find_map(|value| {
                let text = value.as_str()?;
                (EntityKind::from_ref(text) == Some(self.kind)).then(|| text.to_string())
            })?,
        };
        Some(ManagerContextKey {
            org_id: ctx.org_id(),
            entity_kind: self.kind.as_str().to_string(),
            entity_ref,
        })
    }

    /// Hold the change to the manager context revision the caller said it
    /// read: a newer one fails with `manager_context_changed`; context the
    /// caller did not acknowledge leaves a notice naming its revision.
    async fn check_context(&self, ctx: &Ctx) -> Result<(), CommandError> {
        let Some(key) = self.context_key(ctx) else {
            return Ok(());
        };
        // Held to a revision, the read locks the row so a concurrent context
        // write waits for this change's transaction rather than slip under it.
        let read = if self.intent.context_revision.is_some() {
            ctx.db.get_manager_context_for_share(&key).await
        } else {
            ctx.db.get_manager_context(&key).await
        };
        let current = match read {
            Ok(row) => row,
            Err(error) => {
                // Context is guidance, not a lock: failing to read it must not
                // block the change, unless the caller asked to be held to it.
                tracing::warn!(error = %error, "manager context read failed");
                if self.intent.context_revision.is_some() {
                    return Err(CommandError::internal(error));
                }
                return Ok(());
            }
        };
        let revision = current.as_ref().map_or(0, |row| row.revision);
        let entity_ref = &key.entity_ref;
        match self.intent.context_revision {
            Some(read) if read != revision => Err(CommandError::conflict(format!(
                "The manager context of {entity_ref} changed since you read it (revision {read}, \
                 now {revision}). Read it again, check the change still fits, and retry with \
                 --context-revision {revision}"
            ))
            .with_code(context::MANAGER_CONTEXT_CHANGED)
            .with_action(context::reread_action(entity_ref))),
            Some(_) => Ok(()),
            None => {
                if current.is_some_and(|row| !row.content.trim().is_empty()) {
                    self.intent.notices.push(format!(
                        "{entity_ref} has manager context (revision {revision}) this change did \
                         not acknowledge: read it with `everruns context get {entity_ref}` and \
                         pass --context-revision {revision}"
                    ));
                }
                Ok(())
            }
        }
    }

    /// Prepare to record `change` for a command about to run with `params`.
    /// Fails on an invalid reason, before the command can mutate anything.
    pub fn prepare(change: Change, params: Value, ctx: &Ctx) -> Result<Option<Self>, CommandError> {
        let Change::Subject { kind, action, id } = change else {
            return Ok(None);
        };
        let intent = effective_intent(ctx);
        let reason = intent
            .reason
            .as_deref()
            .map(intent::validate_reason)
            .transpose()?;
        if reason.is_none() && ActorKind::of(ctx, &intent) == ActorKind::AgentSession {
            missing_agent_reason(ctx, &intent, kind)?;
        }
        Ok(Some(Self {
            kind,
            action,
            id,
            params,
            intent,
            reason,
        }))
    }

    /// Record the change after the command succeeded with `output`.
    ///
    /// Awaited, not spawned: losing history silently is the failure this
    /// exists to prevent. Inside the command's transaction a failed write
    /// fails the command, which rolls its mutation back. Outside one (a
    /// command that opts out, or a REST route) the mutation has already
    /// committed, so the failure is logged and counted instead.
    /// The output is read before the returned future, so a command output
    /// need not be `Sync` to cross the history write's await.
    pub fn record<'a, T: serde::Serialize>(
        pending: Option<Self>,
        meta: &CommandMeta,
        ctx: &'a Ctx,
        output: &T,
    ) -> impl std::future::Future<Output = Result<(), CommandError>> + Send + 'a {
        let command = meta.name;
        // A deleted entity takes its manager context with it.
        let orphaned_context = pending
            .as_ref()
            .filter(|pending| pending.action == ChangeAction::Deleted)
            .and_then(|pending| pending.context_key(ctx));
        let output = serde_json::to_value(output).unwrap_or(Value::Null);
        let subject = pending
            .as_ref()
            .map(|pending| (pending.kind, pending.action));
        let row = pending.map(|pending| pending.row(meta, ctx, &output));
        async move {
            let atomic = crate::storage::transaction::in_transaction();
            let mut row = match row {
                None => return Ok(()),
                Some(Some(row)) => row,
                Some(None) => {
                    // A declaration bug, not a storage failure: the mutation
                    // stands and the gap is reported.
                    tracing::error!(
                        command,
                        "entity history: the declared subject id is missing from the command"
                    );
                    metrics::counter!(crate::metrics_names::ENTITY_HISTORY_WRITE_FAILURES)
                        .increment(1);
                    return Ok(());
                }
            };
            if let Some((kind, action)) = subject
                && let Some((snapshot, hash)) =
                    snapshot::capture(ctx, kind, action, &row.entity_ref, &output).await
            {
                row.snapshot = Some(snapshot);
                row.snapshot_hash = Some(hash);
            }
            if let Err(error) = ctx.db.record_entity_change(row).await {
                tracing::error!(command, atomic, error = %error, "entity history write failed");
                metrics::counter!(crate::metrics_names::ENTITY_HISTORY_WRITE_FAILURES).increment(1);
                if atomic {
                    return Err(CommandError::internal(error.context(
                        "entity history write failed; the change was rolled back",
                    )));
                }
            }
            if let Some(key) = orphaned_context
                && let Err(error) = ctx.db.delete_manager_context(&key).await
            {
                tracing::warn!(command, error = %error, "manager context cleanup failed");
                if atomic {
                    return Err(CommandError::internal(error));
                }
            }
            Ok(())
        }
    }

    fn row(self, meta: &CommandMeta, ctx: &Ctx, output: &Value) -> Option<NewEntityChange> {
        let entity_ref = match self.id {
            SubjectId::Param(field) => subject_ref(&self.params, field),
            SubjectId::Output(field) => subject_ref(output, field),
        }?;

        let actor_kind = ActorKind::of(ctx, &self.intent);
        let via_session_id = self
            .intent
            .via_session_id
            .or_else(|| ctx.acting_for_session.map(|session| session.uuid()));
        // The update a restore runs records as the restore.
        let action = match (self.intent.restoring, self.action) {
            (Some(_), ChangeAction::Updated) => ChangeAction::Restored,
            (_, action) => action,
        };
        Some(NewEntityChange {
            org_id: ctx.org_id(),
            entity_kind: self.kind.as_str().to_string(),
            entity_ref: entity_ref.clone(),
            command: meta.name.to_string(),
            action: action.as_str().to_string(),
            reason: self.reason,
            changed_fields: changed_fields(&self.params, &entity_ref),
            actor_kind: actor_kind.as_str().to_string(),
            actor_user_id: ctx.caller.user_id,
            via_session_id,
            via_agent_id: self.intent.via_agent_id,
            surface: self
                .intent
                .surface
                .unwrap_or(ChangeSurface::Internal)
                .as_str()
                .to_string(),
            request_id: self.intent.request_id,
            idempotency_key: self.intent.idempotency_key,
            snapshot: None,
            snapshot_hash: None,
            restored_from_revision: self.intent.restoring,
        })
    }
}

/// `#[serde(with = ...)]` for nullable update fields: reads them with
/// `deserialize_nullable_update_field`, and writes them for history only.
///
/// Request params are serialized only to record which fields a change touched,
/// so a cleared field must not read as an absent one: `Clear` writes the
/// string `"<cleared>"`, and `Unchanged` writes `null`, which
/// `changed_fields` skips.
pub mod update_field {
    pub use crate::common_dto::deserialize_nullable_update_field as deserialize;

    pub fn serialize<T, S>(
        field: &crate::storage::UpdateField<T>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        T: serde::Serialize,
        S: serde::Serializer,
    {
        use crate::storage::UpdateField;
        match field {
            UpdateField::Set(value) => value.serialize(serializer),
            UpdateField::Clear => serializer.serialize_str("<cleared>"),
            UpdateField::Unchanged => serializer.serialize_none(),
        }
    }
}

fn subject_ref(value: &Value, field: &str) -> Option<String> {
    match value.get(field)? {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// The fields the caller asked to change: the params' top-level keys that
/// carry a value, minus the one that names the entity. A command that changes
/// a nested object reports it as one field.
fn changed_fields(params: &Value, entity_ref: &str) -> Vec<String> {
    let Some(object) = params.as_object() else {
        return Vec::new();
    };
    let mut fields: Vec<String> = object
        .iter()
        .filter(|(_, value)| !value.is_null() && value.as_str() != Some(entity_ref))
        .map(|(key, _)| key.clone())
        .collect();
    fields.sort();
    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn changed_fields_skip_the_subject_and_unset_values() {
        let params = json!({ "id": "agent_1", "name": "kids", "description": null, "tags": [] });
        assert_eq!(changed_fields(&params, "agent_1"), vec!["name", "tags"]);
        assert_eq!(changed_fields(&json!("scalar"), "x"), Vec::<String>::new());
    }

    #[test]
    fn a_subject_ref_is_a_non_empty_string_or_a_number() {
        assert_eq!(
            subject_ref(&json!({ "id": "agent_1" }), "id").as_deref(),
            Some("agent_1")
        );
        assert_eq!(subject_ref(&json!({ "id": 7 }), "id").as_deref(), Some("7"));
        assert_eq!(subject_ref(&json!({ "id": "" }), "id"), None);
        assert_eq!(subject_ref(&json!({}), "id"), None);
    }
}
