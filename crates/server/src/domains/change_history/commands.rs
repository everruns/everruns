// History read commands: `everruns history list <ref>` and `everruns history org`.
//
// Decision: history is readable exactly where its entity is. The command's own
// policy is only a floor (any org member); the kind's view policy is checked in
// `execute`, because which policy applies depends on the ref. Session history
// additionally requires that the caller can open the session, since session
// visibility is per participant rather than per role.

use chrono::{DateTime, Utc};
use everruns_core::Policy;
use everruns_core::organization::OrgRole;
use everruns_core::permissions::Rule;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use super::registry::EntityKind;
use crate::domains::common::*;
use crate::storage::entity_changes::{EntityChangeQuery, EntityChangeRow};

/// Floor for reading history: any member of the org. The entity kind's own
/// view policy is evaluated per request.
pub const HISTORY_READ: Policy = Policy {
    id: "entity_history.read",
    rules: &[Rule::UserHasRole(OrgRole::Member)],
};

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

/// One recorded change to an entity.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EntityChange {
    /// Id of this history entry.
    pub id: Uuid,
    /// Kind of entity changed, e.g. `agent`.
    #[schema(example = "agent")]
    pub entity_kind: String,
    /// The entity's public id.
    #[schema(example = "agent_01933b5a000070008000000000000001")]
    pub entity_ref: String,
    /// What happened: `created`, `updated`, `deleted`, ...
    #[schema(example = "updated")]
    pub action: String,
    /// Wire name of the command that made the change.
    #[schema(example = "update_agent")]
    pub command: String,
    /// Why, in the caller's words. Caller text, not verified.
    pub reason: Option<String>,
    /// Fields the caller asked to change.
    pub changed_fields: Vec<String>,
    /// `user`, `api_key`, `agent_session` or `system`.
    #[schema(example = "agent_session")]
    pub actor_kind: String,
    /// The user the change was made as (for an agent, the user it acted for).
    pub actor_user_id: Option<Uuid>,
    /// The session through which an agent made the change.
    pub via_session_id: Option<String>,
    /// The agent of that session.
    pub via_agent_id: Option<String>,
    /// Where the change came in: `api`, `commands`, `mcp`, `platform`,
    /// `worker` or `internal`.
    #[schema(example = "platform")]
    pub surface: String,
    /// Correlation id of the HTTP request, when there was one.
    pub request_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl From<EntityChangeRow> for EntityChange {
    fn from(row: EntityChangeRow) -> Self {
        Self {
            id: row.id,
            entity_kind: row.entity_kind,
            entity_ref: row.entity_ref,
            action: row.action,
            command: row.command,
            reason: row.reason,
            changed_fields: row.changed_fields,
            actor_kind: row.actor_kind,
            actor_user_id: row.actor_user_id,
            via_session_id: row
                .via_session_id
                .map(|id| everruns_contracts::typed_id::SessionId::from_uuid(id).to_string()),
            via_agent_id: row.via_agent_id,
            surface: row.surface,
            request_id: row.request_id,
            created_at: row.created_at,
        }
    }
}

fn limit(requested: Option<i64>) -> i64 {
    requested.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

fn parse_kind(kind: &str) -> Result<EntityKind, CommandError> {
    EntityKind::parse(kind).ok_or_else(|| {
        let known: Vec<&str> = EntityKind::ALL.iter().map(|kind| kind.as_str()).collect();
        CommandError::bad_request(format!(
            "Unknown entity kind `{kind}`; expected one of: {}",
            known.join(", ")
        ))
    })
}

fn parse_action(action: Option<String>) -> Result<Option<String>, CommandError> {
    let Some(action) = action else {
        return Ok(None);
    };
    super::registry::ChangeAction::parse(&action)
        .map(|action| Some(action.as_str().to_string()))
        .ok_or_else(|| CommandError::bad_request(format!("Unknown change action `{action}`")))
}

// ============================================================================
// ListEntityHistory
// ============================================================================

/// List the recorded changes to one entity, newest first.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct ListEntityHistory {
    /// The entity's public id, e.g. `agent_01933b5a...`.
    pub entity_ref: String,
    /// Entity kind. Needed only for kinds whose ids have no prefix
    /// (`schedule`, `saved_report`, `check_rule`).
    pub kind: Option<String>,
    /// Only this action (`created`, `updated`, `deleted`, ...).
    pub action: Option<String>,
    /// Only changes older than this timestamp (page cursor).
    pub before: Option<DateTime<Utc>>,
    /// Max entries (default 50, max 200).
    pub limit: Option<i64>,
}

impl Command for ListEntityHistory {
    type Output = Vec<EntityChange>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_entity_history",
            category: "history",
            description: "List recorded changes to one entity (agent, harness, knowledge base, provider, ...), newest first, with who made each change, through which surface and session, and why.",
            method: "GET",
            path: "/v1/history/{entity_ref}",
        }
    }

    fn cli() -> Option<CliRoute> {
        const ROUTE: CliRoute = CliRoute::new(&["history"], "list")
            .with_args(&[CliArg::new("entity_ref").at(1)])
            .with_examples(&[CliExample::new(
                "See why an agent was changed",
                "everruns history list agent_01h9 --limit 10",
            )]);
        Some(ROUTE)
    }

    fn positional_arg() -> Option<&'static str> {
        Some("entity_ref")
    }

    fn policy() -> Option<&'static Policy> {
        Some(&HISTORY_READ)
    }

    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<EntityChange>())
    }

    fn output_shape() -> &'static str {
        "array of {entity_kind, entity_ref, action, reason, changed_fields, actor_kind, via_agent_id, surface, created_at}"
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<EntityChange>, CommandError> {
        let entity_ref = self.entity_ref.trim().to_string();
        if entity_ref.is_empty() {
            return Err(CommandError::bad_request("entity_ref is required"));
        }
        let kind = match self.kind.as_deref() {
            Some(kind) => parse_kind(kind)?,
            None => EntityKind::from_ref(&entity_ref).ok_or_else(|| {
                CommandError::bad_request(format!(
                    "Cannot tell the kind of `{entity_ref}` from its id; pass --kind"
                ))
            })?,
        };
        kind.view_policy()
            .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
            .map_err(|e| CommandError::forbidden(e.message))?;
        if kind == EntityKind::Session {
            // Session visibility is per participant, which no role policy
            // states: the caller must be able to open the session itself.
            // A deleted session's history is reachable through `history org`.
            crate::domains::sessions::commands::GetSession {
                session_id: entity_ref.clone(),
            }
            .execute(ctx)
            .await?;
        }

        let rows = ctx
            .db
            .list_entity_changes(&EntityChangeQuery {
                org_id: ctx.org_id(),
                entity_kind: Some(kind.as_str().to_string()),
                entity_ref: Some(entity_ref),
                action: parse_action(self.action)?,
                before: self.before,
                limit: limit(self.limit),
                ..EntityChangeQuery::default()
            })
            .await
            .map_err(classify_anyhow)?;
        Ok(rows.into_iter().map(EntityChange::from).collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListEntityHistory>() }

// ============================================================================
// ListOrgHistory
// ============================================================================

/// List recorded changes across the organization, newest first.
#[derive(Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct ListOrgHistory {
    /// Only this entity kind.
    pub kind: Option<String>,
    /// Only this action.
    pub action: Option<String>,
    /// Only changes made as this user.
    pub actor_user_id: Option<Uuid>,
    /// Only changes an agent made, by the agent's public id.
    pub via_agent_id: Option<String>,
    /// Only changes at or after this timestamp.
    pub since: Option<DateTime<Utc>>,
    /// Only changes older than this timestamp (page cursor).
    pub before: Option<DateTime<Utc>>,
    /// Max entries (default 50, max 200).
    pub limit: Option<i64>,
}

impl Command for ListOrgHistory {
    type Output = Vec<EntityChange>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_org_history",
            category: "history",
            description: "List recorded changes across the organization, newest first. Filter by entity kind, action, the user a change was made as, or the agent that made it.",
            method: "GET",
            path: "/v1/history",
        }
    }

    fn cli() -> Option<CliRoute> {
        const ROUTE: CliRoute = CliRoute::new(&["history"], "org")
            .with_args(&[CliArg::new("via_agent_id").long("via-agent")])
            .with_examples(&[CliExample::new(
                "See what Platform Chat changed today",
                "everruns history org --via-agent agent_01h9 --since 2026-10-05T00:00:00Z",
            )]);
        Some(ROUTE)
    }

    fn policy() -> Option<&'static Policy> {
        // Changes to every kind at once: the audit view, not any one kind's.
        Some(&crate::domains::audit_logs::AUDIT_LOG_VIEW)
    }

    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<EntityChange>())
    }

    fn output_shape() -> &'static str {
        "array of {entity_kind, entity_ref, action, reason, changed_fields, actor_kind, via_agent_id, surface, created_at}"
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<EntityChange>, CommandError> {
        let kind = self.kind.as_deref().map(parse_kind).transpose()?;
        let rows = ctx
            .db
            .list_entity_changes(&EntityChangeQuery {
                org_id: ctx.org_id(),
                entity_kind: kind.map(|kind| kind.as_str().to_string()),
                entity_ref: None,
                action: parse_action(self.action)?,
                actor_user_id: self.actor_user_id,
                via_agent_id: self.via_agent_id,
                before: self.before,
                since: self.since,
                limit: limit(self.limit),
            })
            .await
            .map_err(classify_anyhow)?;
        Ok(rows.into_iter().map(EntityChange::from).collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListOrgHistory>() }
