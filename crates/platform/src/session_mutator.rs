//! Mutating stored session metadata.
//!
//! `SessionMutator` is the hosted control-plane service behind the `session`
//! capability's title and live-capability-reconfigure tools. It moved out of
//! `everruns-core` with EVE-897: the kernel never mutated a session itself, it
//! only carried the trait on `ToolContext` so a hosted capability could reach
//! it. That capability now resolves [`SessionMutatorExt`] from the type-keyed
//! extension bag instead.
//!
//! Mutations acknowledge with the refreshed portable
//! `everruns_core::ExecutionSession`, never a stored platform record (EVE-882).

use async_trait::async_trait;
use everruns_core::ExecutionSession;
use everruns_core::capability_types::AgentCapabilityConfig;
use everruns_core::error::Result;
use everruns_core::typed_id::SessionId;

/// Trait for updating mutable session metadata.
///
/// Mutations acknowledge with the refreshed portable execution view, never a
/// stored platform record (EVE-882).
#[async_trait]
pub trait SessionMutator: Send + Sync {
    /// Update a session's human-readable title.
    async fn update_session_title(
        &self,
        session_id: SessionId,
        title: String,
    ) -> Result<ExecutionSession>;

    /// Add or replace a session-scoped capability atomically.
    ///
    /// Session backends that support live capability reconfiguration override
    /// this method. The default keeps existing backends source-compatible and
    /// fails closed instead of pretending the capability was applied.
    async fn upsert_session_capability(
        &self,
        _session_id: SessionId,
        _capability: AgentCapabilityConfig,
    ) -> Result<ExecutionSession> {
        Err(everruns_core::error::AgentLoopError::store(
            "session backend does not support live capability reconfiguration",
        ))
    }

    /// Remove a session-scoped capability atomically.
    async fn remove_session_capability(
        &self,
        _session_id: SessionId,
        _capability_id: &str,
    ) -> Result<ExecutionSession> {
        Err(everruns_core::error::AgentLoopError::store(
            "session backend does not support live capability reconfiguration",
        ))
    }
}

#[async_trait]
impl<T: SessionMutator + ?Sized> SessionMutator for std::sync::Arc<T> {
    async fn update_session_title(
        &self,
        session_id: SessionId,
        title: String,
    ) -> Result<ExecutionSession> {
        (**self).update_session_title(session_id, title).await
    }

    async fn upsert_session_capability(
        &self,
        session_id: SessionId,
        capability: AgentCapabilityConfig,
    ) -> Result<ExecutionSession> {
        (**self)
            .upsert_session_capability(session_id, capability)
            .await
    }

    async fn remove_session_capability(
        &self,
        session_id: SessionId,
        capability_id: &str,
    ) -> Result<ExecutionSession> {
        (**self)
            .remove_session_capability(session_id, capability_id)
            .await
    }
}

/// Type-keyed wrapper installed on the tool context by hosted presets.
///
/// Core carries the generic extension bag but does not name this service.
#[derive(Clone)]
pub struct SessionMutatorExt(pub std::sync::Arc<dyn SessionMutator>);
