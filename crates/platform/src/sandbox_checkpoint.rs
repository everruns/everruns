// Logical sandbox and sandbox-checkpoint persistence contract (EVE-870).
//
// The recoverable workspace revision used to live only in
// `provider_state.recovery.head_revision` inside the encrypted `session_sandbox`
// secret. That pointer is written by its own autocommit statement, separately
// from the durable tool result and the `tool.completed` event, so a crash
// between those writes leaves the workspace on a revision no committed tool
// result corresponds to.
//
// This contract gives checkpoints a first-class home so the commit path can
// reason about them: every upload is recorded before it is attached, and only
// an attached checkpoint is authoritative. Uploads that never get attached are
// collectable; attached ones never are.
//
// The trait lives in platform, not core: `everruns-core`'s 0.18 public boundary
// is frozen (EVE-906), and the kernel does not need to name this service. Hosted
// presets install it as a typed `ToolContextExtensions` entry (EVE-897), the
// same shape `SessionSqlDbStore` uses.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use everruns_provider::typed_id::SessionId;

/// How a checkpoint's bytes are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxCheckpointKind {
    /// A provider-managed snapshot. A fast resume path only: it dies with the
    /// provider resource, so it cannot be the recovery path for physical loss.
    ProviderNative,
    /// An Everruns-owned archive of the working filesystem. The recovery path
    /// for physical sandbox loss and provider migration.
    PortableWorkspace,
}

impl SandboxCheckpointKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProviderNative => "provider_native",
            Self::PortableWorkspace => "portable_workspace",
        }
    }
}

/// Identity and fencing state of a logical sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxRef {
    pub id: Uuid,
    /// Current incarnation. Writes carry the generation they were issued
    /// against so a late response from a replaced sandbox cannot advance state.
    pub generation: i64,
}

/// A recorded workspace revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxCheckpoint {
    pub id: Uuid,
    pub sandbox_id: Uuid,
    pub generation: i64,
    pub source_turn_id: Option<String>,
    pub source_tool_call_id: Option<String>,
    pub kind: SandboxCheckpointKind,
    pub provider_ref: Option<String>,
    pub workspace_revision: String,
    /// `None` while the checkpoint is uploaded but not yet authoritative.
    pub attached_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// A checkpoint upload that has completed but is not yet authoritative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSandboxCheckpoint {
    pub sandbox_id: Uuid,
    pub generation: i64,
    pub source_turn_id: Option<String>,
    pub source_tool_call_id: Option<String>,
    pub kind: SandboxCheckpointKind,
    pub provider_ref: Option<String>,
    pub workspace_revision: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SandboxCheckpointError {
    #[error("sandbox checkpoint storage error: {0}")]
    Storage(String),
    /// The write was issued against a superseded incarnation. The caller must
    /// not retry it against the current generation: the bytes it refers to
    /// belong to a sandbox that no longer exists.
    #[error("sandbox {sandbox_id} is at generation {current}, write carried {carried}")]
    StaleGeneration {
        sandbox_id: Uuid,
        current: i64,
        carried: i64,
    },
}

/// Persistence for logical sandboxes and their checkpoints.
#[async_trait]
pub trait SandboxCheckpointStore: Send + Sync {
    /// Resolve the logical sandbox for a session/provider pair, creating it on
    /// first use. Idempotent.
    async fn ensure_sandbox(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<SandboxRef, SandboxCheckpointError>;

    /// Record a completed upload. The checkpoint is not authoritative yet, so a
    /// crash after this point leaves a collectable orphan rather than a pointer
    /// to a revision no committed turn produced.
    ///
    /// Recording the same `workspace_revision` twice returns the existing row.
    async fn record_checkpoint(
        &self,
        checkpoint: NewSandboxCheckpoint,
    ) -> Result<SandboxCheckpoint, SandboxCheckpointError>;

    /// Promote a recorded checkpoint to authoritative, fenced on `generation`.
    /// Returns `StaleGeneration` if the sandbox has been replaced since the
    /// upload, leaving the previous committed checkpoint in place.
    async fn attach_checkpoint(
        &self,
        sandbox_id: Uuid,
        checkpoint_id: Uuid,
        generation: i64,
    ) -> Result<(), SandboxCheckpointError>;

    /// The last checkpoint committed as authoritative, if any.
    async fn current_checkpoint(
        &self,
        sandbox_id: Uuid,
    ) -> Result<Option<SandboxCheckpoint>, SandboxCheckpointError>;

    /// Delete uploads that were never attached and are older than `before`.
    /// Attached checkpoints are never returned or removed. Returns the
    /// `workspace_revision` of each collected row so the caller can drop the
    /// matching provider-side artifact.
    async fn collect_unattached_checkpoints(
        &self,
        sandbox_id: Uuid,
        before: DateTime<Utc>,
    ) -> Result<Vec<String>, SandboxCheckpointError>;
}

/// Type-keyed wrapper installed on the tool context by hosted presets.
///
/// Core carries the generic extension bag but does not name this service, so
/// the store contract stays beside the sandbox code that uses it (EVE-897).
#[derive(Clone)]
pub struct SandboxCheckpointStoreExt(pub std::sync::Arc<dyn SandboxCheckpointStore>);
