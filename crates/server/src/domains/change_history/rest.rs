// History for REST handlers that change an entity without running a command.
//
// Decision: a few REST routes predate the command layer (workspaces CRUD,
// avatars, credential values, the ChatGPT provider connection, skill upload,
// agent import) and write storage directly, so `Command::run` never sees
// them. Until they become commands, each records its change through this
// helper, which runs the same steps the chokepoint does: validate the reason
// and the acknowledged manager context revision before the write, record the
// entry (and drop a deleted entity's context) after it. The entry's command
// is the handler's operation id, so history still says what ran.

use std::sync::Arc;

use serde_json::{Value, json};

use super::{Change, ChangeAction, EntityKind, PendingChange, SubjectId};
use crate::domains::common::{CommandError, CommandMeta, Ctx};
use crate::storage::StorageBackend;
use everruns_core::Caller;

/// A REST change in progress: checked, not yet recorded.
pub struct RestChange {
    ctx: Ctx,
    operation: &'static str,
    pending: Option<PendingChange>,
}

impl RestChange {
    /// Before the write: validate the request's reason and, for an existing
    /// entity, the manager context revision it acknowledged. `fields` are the
    /// names of what the request changes.
    pub async fn begin(
        db: Arc<StorageBackend>,
        caller: Caller,
        operation: &'static str,
        kind: EntityKind,
        action: ChangeAction,
        entity_ref: Option<&str>,
        fields: &[&str],
    ) -> Result<Self, CommandError> {
        let ctx = Ctx::minimal(
            caller,
            db,
            None,
            Arc::new(everruns_core::DefaultPermissionResolver),
        );
        let mut params = serde_json::Map::new();
        for field in fields {
            params.insert((*field).to_string(), Value::Bool(true));
        }
        if let Some(entity_ref) = entity_ref {
            params.insert("id".to_string(), entity_ref.into());
        }
        let change = Change::Subject {
            kind,
            action,
            id: SubjectId::Output("id"),
        };
        let pending = PendingChange::prepare(change, Value::Object(params), &ctx)?;
        if let Some(pending) = &pending {
            pending.check_context(&ctx).await?;
        }
        Ok(Self {
            ctx,
            operation,
            pending,
        })
    }

    /// After the write succeeded: record the change to `entity_ref`.
    pub async fn finish(self, entity_ref: &str) {
        let meta = CommandMeta {
            name: self.operation,
            category: "rest",
            description: "",
            method: "",
            path: "",
        };
        PendingChange::record(self.pending, &meta, &self.ctx, &json!({ "id": entity_ref })).await;
    }
}
