// Storage backend abstraction
// Decision: Use enum dispatch for simplicity over trait objects
//
// This module provides a unified StorageBackend enum that can work with
// either PostgreSQL (production) or in-memory (dev mode) storage.

use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_core::message_filter::MessageQuery;
use everruns_provider::typed_id::{
    AgentId, AgentIdentityId, EventId, HarnessId, KnowledgeBaseId, KnowledgeEntryId,
    KnowledgeIndexId, LeasedResourceId, MemoryId, MessageId, NotificationId, PrincipalId,
    ScheduleId, SessionId, SessionParticipantId, TriggerId, WorkspaceId,
};
use sqlx::PgPool;
use uuid::Uuid;

pub const USER_PREFERENCE_LIMIT_EXCEEDED: &str = "user preference limit exceeded";

/// The error a forced storage failure raises: shaped like a real sqlx/Postgres error,
/// so a test can assert that none of it survives the trip to a client.
#[cfg(test)]
pub(crate) const FORCED_STORAGE_FAILURE: &str = "error returned from database: relation \
     \"agents\" does not exist at sqlx-postgres-0.8.6/src/connection/mod.rs:666";

use super::IngressEndpointRow;
use super::mcp_tool_cache::*;
use super::memory::InMemoryDatabase;
use super::models::*;
use super::reporting::models::ReportingOutboxRow;
use super::repositories::Database;
use crate::api::common::Pagination;

/// Hard upper bound on a single retention-prune batch (EVE-580). Caps the
/// destructive `prune_terminal_session_tasks_with_artifacts` regardless of
/// caller input so a misconfigured limit can never request an unbounded or
/// oversized delete; large backlogs drain over successive reaper ticks.
const MAX_RETENTION_PRUNE_LIMIT: i64 = 1000;

/// Hard cap for participant history returned in one session response.
/// Storage queries fetch one extra row so callers can reject oversized histories
/// instead of allocating or serializing unbounded attacker-created rows.
pub const MAX_SESSION_PARTICIPANT_HISTORY: usize = 512;

const TASK_ARTIFACT_ROOTS: &[&str] = &["/.tasks", "/.background", "/.agent-runs"];

fn task_artifact_delete_root(result_path: &str) -> Option<&str> {
    if !result_path.starts_with('/') || result_path.contains("..") || result_path.contains("//") {
        return None;
    }

    let mut parts = result_path.split('/');
    if parts.next() != Some("") {
        return None;
    }
    let root_name = parts.next()?;
    let run_id = parts.next()?;

    if root_name.is_empty() || run_id.is_empty() || parts.next().is_none() {
        return None;
    }

    let root = match root_name {
        ".tasks" => "/.tasks",
        ".background" => "/.background",
        ".agent-runs" => "/.agent-runs",
        _ => return None,
    };
    if !TASK_ARTIFACT_ROOTS.contains(&root) {
        return None;
    }

    Some(&result_path[..root.len() + 1 + run_id.len()])
}

/// Helper macro to dispatch method calls to the appropriate backend.
///
/// This reduces the repetitive match pattern from 4 lines to 1 line per method.
///
/// # Usage
///
/// ```ignore
/// pub async fn method_name(&self, arg1: T1, arg2: T2) -> Result<R> {
///     dispatch!(self, method_name, arg1, arg2)
/// }
/// ```
macro_rules! dispatch {
    ($self:ident, $method:ident $(, $arg:expr)*) => {
        match $self {
            Self::Postgres(db) => db.$method($($arg),*).await,
            Self::InMemory(db) => db.$method($($arg),*).await,
        }
    };
}

/// Storage backend that can be either PostgreSQL or in-memory
#[derive(Clone)]
pub enum StorageBackend {
    /// PostgreSQL database (production)
    Postgres(Database),
    /// In-memory database (dev mode)
    InMemory(std::sync::Arc<InMemoryDatabase>),
}

mod harnesses_sessions;
mod identity;
mod knowledge;
mod models_files;
mod observers_billing;
mod orgs_images;
mod resources_tasks;

#[cfg(test)]
mod retention_tests {
    use super::*;
    use everruns_core::session_task::{
        CreateSessionTask, SessionTaskRegistry, SessionTaskState, SessionTaskUpdate, TaskLinks,
        TaskWakePolicy,
    };
    use everruns_provider::typed_id::SessionId;
    use std::sync::Arc;

    // The retention prune deletes a task's recorded internal artifact subtree
    // through the existing session-file deletion seam after the row commits
    // (EVE-580). Proven against the in-memory backend: a terminal task with a
    // result_path has its row removed AND its artifact file deleted, while a
    // live task and its file are untouched.
    #[tokio::test]
    async fn prune_with_artifacts_deletes_rows_and_artifact_files() {
        let db = Arc::new(StorageBackend::in_memory());
        let registry = crate::storage::DbSessionTaskRegistry::new(db.clone());
        let session_id = SessionId::new();
        let sid = session_id.uuid();

        // Terminal task with an artifact file under a production background artifact path.
        let terminal = registry
            .create(CreateSessionTask {
                session_id,
                id: Some("task_term".to_string()),
                kind: "background_tool".to_string(),
                display_name: "done".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();
        registry
            .update(
                session_id,
                &terminal.id,
                SessionTaskUpdate {
                    state: Some(SessionTaskState::Succeeded),
                    result_path: Some("/.background/bg_task_term/result.json".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        db.create_session_file(crate::storage::models::CreateSessionFileRow {
            session_id,
            path: "/.background/bg_task_term/result.json".to_string(),
            content: Some(b"{}".to_vec()),
            is_directory: false,
            is_readonly: false,
        })
        .await
        .unwrap();

        // Live task with a file — must survive.
        registry
            .create(CreateSessionTask {
                session_id,
                id: Some("task_live".to_string()),
                kind: "background_tool".to_string(),
                display_name: "live".to_string(),
                spec: serde_json::json!({}),
                state: SessionTaskState::Running,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            })
            .await
            .unwrap();
        db.create_session_file(crate::storage::models::CreateSessionFileRow {
            session_id,
            path: "/.background/bg_task_live/result.json".to_string(),
            content: Some(b"{}".to_vec()),
            is_directory: false,
            is_readonly: false,
        })
        .await
        .unwrap();

        // Negative TTL → cutoff just after now, so the just-finished terminal
        // task is eligible; the running task can never be (state guard).
        let pruned = db
            .prune_terminal_session_tasks_with_artifacts(chrono::Duration::seconds(-1), 100)
            .await
            .unwrap();
        assert_eq!(pruned, 1, "only the terminal task is pruned");

        // Terminal row + its artifact are gone.
        assert!(
            registry
                .get(session_id, "task_term")
                .await
                .unwrap()
                .is_none(),
            "terminal task row removed"
        );
        assert!(
            db.get_session_file(sid, "/.background/bg_task_term/result.json")
                .await
                .unwrap()
                .is_none(),
            "terminal task artifact deleted via the session-file seam"
        );

        // Live row + its artifact survive.
        assert!(
            registry
                .get(session_id, "task_live")
                .await
                .unwrap()
                .is_some(),
            "live task untouched"
        );
        assert!(
            db.get_session_file(sid, "/.background/bg_task_live/result.json")
                .await
                .unwrap()
                .is_some(),
            "live task artifact untouched"
        );
    }
}
