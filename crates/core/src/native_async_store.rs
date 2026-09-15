//! Shared ownership and checkpoint persistence for native tool conversations.
//! The lease token fences every read/write; a worker must renew before expiry.

use async_trait::async_trait;
use everruns_provider::{
    error::Result,
    native_async::NativeAsyncCheckpoint,
    typed_id::{SessionId, TurnId},
};
use uuid::Uuid;

/// A lease belongs to one turn in one tenant. Tokens are never reused by a new owner.
#[derive(Debug, Clone, Copy)]
pub struct NativeAsyncLease {
    /// Owning tenant identifier.
    pub org_id: i64,
    /// Owning conversation identifier.
    pub session_id: SessionId,
    /// Turn whose provider calls are checkpointed.
    pub turn_id: TurnId,
    /// Unique fencing token for this acquisition.
    pub owner: Uuid,
}

/// Maximum time a worker may retain ownership without renewal.
pub const NATIVE_ASYNC_LEASE_SECONDS: i64 = 60;
/// Checkpoint size bound below the internal RPC transport limit.
pub const MAX_NATIVE_ASYNC_CHECKPOINT_BYTES: usize = 8 * 1024 * 1024;

/// Durable, tenant-scoped storage with exclusive expiring ownership.
#[async_trait]
pub trait NativeAsyncStore: Send + Sync {
    /// Acquire an absent/expired lease, preserving the previous owner's journal.
    /// Retrying acquisition with the same live token is idempotent.
    async fn acquire(&self, lease: NativeAsyncLease) -> Result<NativeAsyncCheckpoint>;
    /// Read only under a live ownership fence.
    async fn load(&self, lease: NativeAsyncLease) -> Result<NativeAsyncCheckpoint>;
    /// Renew the live lease; an expired owner cannot resurrect itself.
    async fn renew(&self, lease: NativeAsyncLease) -> Result<()>;
    /// Atomically persist the checkpoint and renew the live lease.
    async fn save(&self, lease: NativeAsyncLease, checkpoint: &NativeAsyncCheckpoint)
    -> Result<()>;
    /// Keep the journal and tombstones; release only this owner's live lease.
    async fn release(&self, lease: NativeAsyncLease) -> Result<()>;
}
