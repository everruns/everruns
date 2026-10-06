//! Entity history rows (`entity_changes`). See
//! `crate::domains::change_history` for what writes and reads them; the
//! storage only appends and lists.

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// One change to record.
#[derive(Debug, Clone, PartialEq)]
pub struct NewEntityChange {
    pub org_id: i64,
    pub entity_kind: String,
    pub entity_ref: String,
    pub command: String,
    pub action: String,
    pub reason: Option<String>,
    pub changed_fields: Vec<String>,
    pub actor_kind: String,
    pub actor_user_id: Option<Uuid>,
    pub via_session_id: Option<Uuid>,
    pub via_agent_id: Option<String>,
    pub surface: String,
    pub request_id: Option<String>,
    pub idempotency_key: Option<String>,
    /// The entity as it stood after the change, secrets as markers. `None`
    /// for kinds without snapshots and for deletes.
    pub snapshot: Option<serde_json::Value>,
    /// Hash of `snapshot`; an entry whose hash matches the entity's latest
    /// revision is a no-op and gets no revision of its own.
    pub snapshot_hash: Option<String>,
    /// The revision a restore brought back.
    pub restored_from_revision: Option<i64>,
}

/// Snapshots kept per entity. Older revisions keep their entry and reason
/// and lose only the snapshot.
pub const MAX_SNAPSHOTS_PER_ENTITY: i64 = 500;

/// A revision: an entry and the snapshot it carries (`None` once pruned).
#[derive(Debug, Clone, PartialEq)]
pub struct EntityRevisionRow {
    pub entry: EntityChangeRow,
    pub snapshot: Option<serde_json::Value>,
}

/// Which revision of which entity to read; `revision: None` is the latest.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityRevisionKey {
    pub org_id: i64,
    pub entity_kind: String,
    pub entity_ref: String,
    pub revision: Option<i64>,
}

/// What the in-memory store keeps per entry.
#[derive(Debug, Clone)]
pub struct StoredEntityChange {
    pub row: EntityChangeRow,
    pub snapshot: Option<serde_json::Value>,
    pub snapshot_hash: Option<String>,
}

/// A recorded change.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct EntityChangeRow {
    pub id: Uuid,
    pub org_id: i64,
    pub entity_kind: String,
    pub entity_ref: String,
    pub command: String,
    pub action: String,
    pub reason: Option<String>,
    pub changed_fields: Vec<String>,
    pub actor_kind: String,
    pub actor_user_id: Option<Uuid>,
    pub via_session_id: Option<Uuid>,
    pub via_agent_id: Option<String>,
    pub surface: String,
    pub request_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub revision: Option<i64>,
    pub restored_from_revision: Option<i64>,
    pub created_at: DateTime<Utc>,
}

impl EntityChangeRow {
    pub fn from_new(id: Uuid, created_at: DateTime<Utc>, change: NewEntityChange) -> Self {
        Self {
            id,
            org_id: change.org_id,
            entity_kind: change.entity_kind,
            entity_ref: change.entity_ref,
            command: change.command,
            action: change.action,
            reason: change.reason,
            changed_fields: change.changed_fields,
            actor_kind: change.actor_kind,
            actor_user_id: change.actor_user_id,
            via_session_id: change.via_session_id,
            via_agent_id: change.via_agent_id,
            surface: change.surface,
            request_id: change.request_id,
            idempotency_key: change.idempotency_key,
            revision: None,
            restored_from_revision: change.restored_from_revision,
            created_at,
        }
    }
}

/// Which changes to list, newest first. Always scoped to one org.
#[derive(Debug, Clone, Default)]
pub struct EntityChangeQuery {
    pub org_id: i64,
    pub entity_kind: Option<String>,
    pub entity_ref: Option<String>,
    pub action: Option<String>,
    pub actor_user_id: Option<Uuid>,
    pub via_agent_id: Option<String>,
    /// Only changes strictly older than this (a page cursor).
    pub before: Option<DateTime<Utc>>,
    pub since: Option<DateTime<Utc>>,
    pub limit: i64,
}

impl EntityChangeQuery {
    /// Whether `row` passes every filter but the limit; the in-memory store
    /// uses it, and it states the PostgreSQL query's meaning in one place.
    pub fn matches(&self, row: &EntityChangeRow) -> bool {
        row.org_id == self.org_id
            && self
                .entity_kind
                .as_ref()
                .is_none_or(|k| *k == row.entity_kind)
            && self
                .entity_ref
                .as_ref()
                .is_none_or(|r| *r == row.entity_ref)
            && self.action.as_ref().is_none_or(|a| *a == row.action)
            && self
                .actor_user_id
                .is_none_or(|actor| row.actor_user_id == Some(actor))
            && self
                .via_agent_id
                .as_ref()
                .is_none_or(|agent| row.via_agent_id.as_ref() == Some(agent))
            && self.before.is_none_or(|before| row.created_at < before)
            && self.since.is_none_or(|since| row.created_at >= since)
    }
}
