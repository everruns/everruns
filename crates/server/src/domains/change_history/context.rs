// Manager context commands: `everruns context get|set|append|clear <ref>`.
//
// Decision: context is a separate record, never a field on the entity, so it
// stays out of everything the entity's runtime reads (prompt, resolved config,
// exports, previews, events) by construction. Reading it needs the kind's
// manage policy: notes addressed to managers are for the people who can act on
// them. The command's own policy is only a floor; the kind's policy is checked
// in `execute`, because which one applies depends on the ref.
//
// Decision (self rule): a session acting for an entity cannot read or write
// that entity's context or history. An agent that can read the constraints set
// on it can argue with them, and its prompt budget is not the place for them.
// Other entities' context stays reachable when the caller manages them.
//
// Writes are entity changes of the entity they describe (`context_updated`),
// but the kind is known only from the ref, so these commands are exempt from
// the static registry and record their entry through the same
// `PendingChange` the chokepoint uses: the reason is validated before the
// write and recorded after it.

use chrono::{DateTime, Utc};
use everruns_core::Policy;
use everruns_core::organization::OrgRole;
use everruns_core::permissions::Rule;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use super::registry::{Change, ChangeAction, EntityKind, SubjectId};
use super::{PendingChange, effective_intent};
use crate::api::common::AllowedAction;
use crate::domains::common::*;
use crate::storage::manager_context::{
    ManagerContextEdit, ManagerContextKey, ManagerContextRow, ManagerContextWriteError,
};

/// Error code: the entity's manager context changed since the caller read it.
pub const MANAGER_CONTEXT_CHANGED: &str = "manager_context_changed";

/// Floor for the context commands: any member of the org. The entity kind's
/// manage policy is evaluated per request.
pub const MANAGER_CONTEXT_ACCESS: Policy = Policy {
    id: "manager_context.access",
    rules: &[Rule::UserHasRole(OrgRole::Member)],
};

/// The recovery step for a stale context revision: read the context again.
pub fn reread_action(entity_ref: &str) -> AllowedAction {
    AllowedAction::new("retry")
        .with_operation_id("get_manager_context")
        .with_hint(format!(
            "Run `everruns context get {entity_ref}`, check the change still fits the notes, \
             then retry with --context-revision set to the revision it returns."
        ))
}

/// An entity's manager context.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ManagerContext {
    #[schema(example = "agent")]
    pub entity_kind: String,
    #[schema(example = "agent_01933b5a000070008000000000000001")]
    pub entity_ref: String,
    /// Markdown, at most 16 KiB. Empty when none was written or it was cleared.
    pub content: String,
    /// Grows by one per write; 0 when none was ever written. Pass it as
    /// `--expected-revision` to a context write or `--context-revision` to a
    /// change of the entity.
    pub revision: i64,
    /// The user the last write was made as.
    pub updated_by_user_id: Option<Uuid>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl ManagerContext {
    fn of(kind: EntityKind, entity_ref: String, row: Option<ManagerContextRow>) -> Self {
        match row {
            Some(row) => Self {
                entity_kind: row.entity_kind,
                entity_ref: row.entity_ref,
                content: row.content,
                revision: row.revision,
                updated_by_user_id: row.updated_by_user_id,
                updated_at: Some(row.updated_at),
            },
            None => Self {
                entity_kind: kind.as_str().to_string(),
                entity_ref,
                content: String::new(),
                revision: 0,
                updated_by_user_id: None,
                updated_at: None,
            },
        }
    }
}

/// The kind `entity_ref` names, from its prefix or an explicit `kind`.
pub(super) fn resolve_kind(
    entity_ref: &str,
    kind: Option<&str>,
) -> Result<EntityKind, CommandError> {
    if entity_ref.is_empty() {
        return Err(CommandError::bad_request("entity_ref is required"));
    }
    match kind {
        Some(kind) => super::commands::parse_kind(kind),
        None => EntityKind::from_ref(entity_ref).ok_or_else(|| {
            CommandError::bad_request(format!(
                "Cannot tell the kind of `{entity_ref}` from its id; pass --kind"
            ))
        }),
    }
}

/// Refuse when the caller is a session acting for `entity_ref` itself.
// THREAT[TM-AGENT-034]: an agent must not read or rewrite the constraints its
// managers set on it.
pub(super) async fn deny_self(ctx: &Ctx, entity_ref: &str) -> Result<(), CommandError> {
    let intent = effective_intent(ctx);
    let mut own = Vec::new();
    if let Some(agent) = intent.via_agent_id {
        own.push(agent);
    }
    let session = ctx.acting_for_session.or(intent
        .via_session_id
        .map(everruns_contracts::typed_id::SessionId::from_uuid));
    if let Some(session) = session
        && let Ok(Some(row)) = ctx.db.get_session(ctx.org_id(), session).await
    {
        own.extend(row.agent_id.map(|agent| agent.to_string()));
        own.extend(row.harness_id.map(|harness| harness.to_string()));
    }
    if own.iter().any(|id| id == entity_ref) {
        return Err(CommandError::forbidden(format!(
            "A session acting for {entity_ref} cannot read or change its own manager context or \
             history; ask a manager of {entity_ref} instead"
        ))
        .with_code("self_inspection_denied"));
    }
    Ok(())
}

/// Resolve the entity, check the caller manages its kind and is not the
/// entity's own runtime, and confirm the entity exists and is visible.
async fn managed_entity(
    ctx: &Ctx,
    entity_ref: &str,
    kind: Option<&str>,
) -> Result<ManagerContextKey, CommandError> {
    let entity_ref = entity_ref.trim();
    let kind = resolve_kind(entity_ref, kind)?;
    if !kind.has_manager_context() {
        return Err(CommandError::bad_request(format!(
            "{} entities have no manager context; keep notes on their parent",
            kind.as_str()
        )));
    }
    kind.manage_policy()
        .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
        .map_err(|e| CommandError::forbidden(e.message))?;
    deny_self(ctx, entity_ref).await?;
    if let Some((read, param)) = kind.lookup() {
        let mut params = serde_json::Map::new();
        params.insert(param.to_string(), entity_ref.into());
        dispatch(read, params.into(), ctx).await?;
    }
    Ok(ManagerContextKey {
        org_id: ctx.org_id(),
        entity_kind: kind.as_str().to_string(),
        entity_ref: entity_ref.to_string(),
    })
}

/// Apply `edit` to the context of `entity_ref`, recording a `context_updated`
/// history entry with the caller's reason.
async fn write(
    ctx: &Ctx,
    meta: &CommandMeta,
    entity_ref: &str,
    kind: Option<&str>,
    edit: ManagerContextEdit,
    expected_revision: Option<i64>,
) -> Result<ManagerContext, CommandError> {
    let key = managed_entity(ctx, entity_ref, kind).await?;
    let entity_kind = EntityKind::parse(&key.entity_kind).expect("resolved above");
    let change = Change::Subject {
        kind: entity_kind,
        action: ChangeAction::ContextUpdated,
        id: SubjectId::Param("entity_ref"),
    };
    let params = json!({ "entity_ref": key.entity_ref, "manager_context": true });
    let pending = PendingChange::prepare(change, params, ctx)?;
    let row = ctx
        .db
        .write_manager_context(&key, &edit, expected_revision, ctx.caller.user_id)
        .await
        .map_err(|error| match error {
            ManagerContextWriteError::Stale { expected, current } => {
                CommandError::conflict(format!(
                    "The manager context of {} is at revision {current}, not {expected}; read it \
                 again and merge your edit",
                    key.entity_ref
                ))
                .with_code(MANAGER_CONTEXT_CHANGED)
                .with_action(reread_action(&key.entity_ref))
            }
            ManagerContextWriteError::TooLarge(_) => CommandError::unprocessable(error.to_string())
                .with_code("manager_context_too_large"),
            ManagerContextWriteError::Storage(error) => CommandError::internal(error),
        })?;
    let output = ManagerContext::of(entity_kind, key.entity_ref, Some(row));
    PendingChange::record(pending, meta, ctx, &output).await;
    Ok(output)
}

fn context_output_schema() -> serde_json::Value {
    output_schema_for::<ManagerContext>()
}

const OUTPUT_SHAPE: &str =
    "{entity_kind, entity_ref, content, revision, updated_by_user_id, updated_at}";
const EXEMPT: Change = Change::Exempt(
    "records the context_updated entry of the entity it names itself, since the kind comes \
     from the ref",
);

// ============================================================================
// GetManagerContext
// ============================================================================

/// Read an entity's manager context.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct GetManagerContext {
    /// The entity's public id, e.g. `agent_01933b5a...`.
    pub entity_ref: String,
    /// Entity kind, needed only for ids without a prefix (`schedule`,
    /// `saved_report`, `check_rule`).
    pub kind: Option<String>,
}

impl Command for GetManagerContext {
    type Output = ManagerContext;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_manager_context",
            category: "context",
            description: "Read the manager context of an entity: notes its managers keep about it (requirements, rationale, ownership). Read it before changing the entity and pass its revision as --context-revision.",
            method: "GET",
            path: "/v1/context/{entity_ref}",
        }
    }

    fn cli() -> Option<CliRoute> {
        const ROUTE: CliRoute = CliRoute::new(&["context"], "get")
            .with_args(&[CliArg::new("entity_ref").at(1)])
            .with_examples(&[CliExample::new(
                "Read what an agent's managers require before changing it",
                "everruns context get agent_01h9",
            )]);
        Some(ROUTE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("entity_ref")
    }

    fn policy() -> Option<&'static Policy> {
        Some(&MANAGER_CONTEXT_ACCESS)
    }

    fn output_schema() -> serde_json::Value {
        context_output_schema()
    }

    fn output_shape() -> &'static str {
        OUTPUT_SHAPE
    }

    async fn execute(self, ctx: &Ctx) -> Result<ManagerContext, CommandError> {
        let key = managed_entity(ctx, &self.entity_ref, self.kind.as_deref()).await?;
        let row = ctx
            .db
            .get_manager_context(&key)
            .await
            .map_err(classify_anyhow)?;
        let kind = EntityKind::parse(&key.entity_kind).expect("resolved above");
        Ok(ManagerContext::of(kind, key.entity_ref, row))
    }
}

inventory::submit! { CommandDescriptor::of::<GetManagerContext>() }

// ============================================================================
// SetManagerContext
// ============================================================================

/// Replace an entity's manager context.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct SetManagerContext {
    /// The entity's public id.
    pub entity_ref: String,
    /// Entity kind, needed only for ids without a prefix.
    pub kind: Option<String>,
    /// The whole document, markdown, at most 16 KiB.
    pub content: String,
    /// The revision this edit was based on; refused when the stored one
    /// differs. 0 when the entity has none yet.
    pub expected_revision: Option<i64>,
}

impl Command for SetManagerContext {
    type Output = ManagerContext;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "set_manager_context",
            category: "context",
            description: "Replace the manager context of an entity with a new markdown document. Pass --expected-revision to refuse the write if someone changed it since you read it.",
            method: "PUT",
            path: "/v1/context/{entity_ref}",
        }
    }

    fn cli() -> Option<CliRoute> {
        const ROUTE: CliRoute = CliRoute::new(&["context"], "set")
            .with_args(&[CliArg::new("entity_ref").at(1)])
            .with_examples(&[CliExample::new(
                "Record an agent's requirements from a file",
                "everruns context set agent_01h9 --content @notes.md --expected-revision 3 --reason 'Product review decisions'",
            )]);
        Some(ROUTE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("entity_ref")
    }

    fn policy() -> Option<&'static Policy> {
        Some(&MANAGER_CONTEXT_ACCESS)
    }

    fn change() -> Change {
        EXEMPT
    }

    fn output_schema() -> serde_json::Value {
        context_output_schema()
    }

    fn output_shape() -> &'static str {
        OUTPUT_SHAPE
    }

    async fn execute(self, ctx: &Ctx) -> Result<ManagerContext, CommandError> {
        let edit = ManagerContextEdit::Set(self.content);
        let (entity_ref, kind) = (self.entity_ref, self.kind);
        write(
            ctx,
            &Self::meta(),
            &entity_ref,
            kind.as_deref(),
            edit,
            self.expected_revision,
        )
        .await
    }
}

inventory::submit! { CommandDescriptor::of::<SetManagerContext>() }

// ============================================================================
// AppendManagerContext
// ============================================================================

/// Add a paragraph to the end of an entity's manager context.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct AppendManagerContext {
    /// The entity's public id.
    pub entity_ref: String,
    /// Entity kind, needed only for ids without a prefix.
    pub kind: Option<String>,
    /// The paragraph to add, markdown.
    pub text: String,
}

impl Command for AppendManagerContext {
    type Output = ManagerContext;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "append_manager_context",
            category: "context",
            description: "Add a paragraph to the end of an entity's manager context, such as a requirement a user stated. Needs no prior read.",
            method: "POST",
            path: "/v1/context/{entity_ref}/append",
        }
    }

    fn cli() -> Option<CliRoute> {
        const ROUTE: CliRoute = CliRoute::new(&["context"], "append")
            .with_args(&[CliArg::new("entity_ref").at(1)])
            .with_examples(&[CliExample::new(
                "Record a requirement a user stated about an agent",
                "everruns context append agent_01h9 --text 'Answers must stay suitable for children.' --reason 'User asked in Platform Chat'",
            )]);
        Some(ROUTE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("entity_ref")
    }

    fn policy() -> Option<&'static Policy> {
        Some(&MANAGER_CONTEXT_ACCESS)
    }

    fn change() -> Change {
        EXEMPT
    }

    fn output_schema() -> serde_json::Value {
        context_output_schema()
    }

    fn output_shape() -> &'static str {
        OUTPUT_SHAPE
    }

    async fn execute(self, ctx: &Ctx) -> Result<ManagerContext, CommandError> {
        if self.text.trim().is_empty() {
            return Err(CommandError::bad_request("text must not be empty"));
        }
        let edit = ManagerContextEdit::Append(self.text.trim().to_string());
        write(
            ctx,
            &Self::meta(),
            &self.entity_ref,
            self.kind.as_deref(),
            edit,
            None,
        )
        .await
    }
}

inventory::submit! { CommandDescriptor::of::<AppendManagerContext>() }

// ============================================================================
// ClearManagerContext
// ============================================================================

/// Empty an entity's manager context. Its revision keeps growing.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct ClearManagerContext {
    /// The entity's public id.
    pub entity_ref: String,
    /// Entity kind, needed only for ids without a prefix.
    pub kind: Option<String>,
    /// The revision being cleared; refused when the stored one differs.
    pub expected_revision: Option<i64>,
}

impl Command for ClearManagerContext {
    type Output = ManagerContext;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "clear_manager_context",
            category: "context",
            description: "Empty the manager context of an entity. The cleared text stays in no history; record why with --reason.",
            method: "DELETE",
            path: "/v1/context/{entity_ref}",
        }
    }

    fn cli() -> Option<CliRoute> {
        const ROUTE: CliRoute = CliRoute::new(&["context"], "clear")
            .with_args(&[CliArg::new("entity_ref").at(1)])
            .with_examples(&[CliExample::new(
                "Drop notes that no longer apply",
                "everruns context clear agent_01h9 --expected-revision 4 --reason 'Requirements moved to the harness'",
            )]);
        Some(ROUTE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("entity_ref")
    }

    fn policy() -> Option<&'static Policy> {
        Some(&MANAGER_CONTEXT_ACCESS)
    }

    fn change() -> Change {
        EXEMPT
    }

    fn output_schema() -> serde_json::Value {
        context_output_schema()
    }

    fn output_shape() -> &'static str {
        OUTPUT_SHAPE
    }

    async fn execute(self, ctx: &Ctx) -> Result<ManagerContext, CommandError> {
        let edit = ManagerContextEdit::Clear;
        let (entity_ref, kind) = (self.entity_ref, self.kind);
        write(
            ctx,
            &Self::meta(),
            &entity_ref,
            kind.as_deref(),
            edit,
            self.expected_revision,
        )
        .await
    }
}

inventory::submit! { CommandDescriptor::of::<ClearManagerContext>() }
