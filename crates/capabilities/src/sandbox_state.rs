//! Durable logical-sandbox and physical-incarnation state.
//!
//! Provider credentials never belong here. The store persists only the
//! provider-neutral lifecycle record and the opaque, non-secret state a driver
//! needs to reconnect to or replace one physical incarnation.

use async_trait::async_trait;
use thiserror::Error;

use everruns_contracts::typed_id::SessionId;

use crate::sandbox_checkpoint::SandboxRef;
use crate::session_sandbox::{SESSION_SANDBOX_SECRET_NAME, SessionSandboxState};
use everruns_core::{tool_context::ToolContext, tools::ToolExecutionResult};

pub const MAX_SANDBOX_STATE_FIELD_BYTES: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum SandboxStateError {
    #[error("sandbox state storage error: {0}")]
    Storage(String),
    #[error("sandbox {sandbox_id} is at generation {current}, write carried {carried}")]
    StaleGeneration {
        sandbox_id: uuid::Uuid,
        current: i64,
        carried: i64,
    },
    #[error("sandbox write carried {carried}, current logical sandbox is {current}")]
    WrongSandbox {
        current: uuid::Uuid,
        carried: uuid::Uuid,
    },
    #[error("sandbox {field} is {size} bytes; maximum is {limit}")]
    StateTooLarge {
        field: &'static str,
        size: usize,
        limit: usize,
    },
}

pub fn validate_sandbox_state(state: &SessionSandboxState) -> Result<(), SandboxStateError> {
    for (field, value) in [
        ("provider_state", &state.instance.provider_state),
        ("metadata", &state.instance.metadata),
    ] {
        let size = serde_json::to_vec(value)
            .map_err(|error| SandboxStateError::Storage(error.to_string()))?
            .len();
        if size > MAX_SANDBOX_STATE_FIELD_BYTES {
            return Err(SandboxStateError::StateTooLarge {
                field,
                size,
                limit: MAX_SANDBOX_STATE_FIELD_BYTES,
            });
        }
    }
    Ok(())
}

/// Persistence for the current physical incarnation of a logical sandbox.
#[async_trait]
pub trait SandboxStateStore: Send + Sync {
    /// Load the session's current logical sandbox. Session scope permits at
    /// most one; a second provider is rejected by product configuration.
    async fn load_current_state(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionSandboxState>, SandboxStateError>;

    /// Load the current incarnation for a session/provider pair.
    async fn load_state(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<SessionSandboxState>, SandboxStateError>;

    /// Persist the current incarnation. Changing `external_id` advances the
    /// logical sandbox generation and retires the previous incarnation.
    async fn save_state(
        &self,
        session_id: SessionId,
        state: &SessionSandboxState,
        expected: Option<&SandboxRef>,
    ) -> Result<SandboxRef, SandboxStateError>;

    /// Delete the logical sandbox and all subordinate incarnation/checkpoint
    /// records after the provider resource has been released.
    async fn delete_state(
        &self,
        session_id: SessionId,
        provider: &str,
        expected: Option<&SandboxRef>,
    ) -> Result<bool, SandboxStateError>;
}

/// Hosted store installed on [`everruns_core::ToolContext`] as a typed
/// extension. Framework/remote hosts that do not install it retain the legacy
/// session-secret fallback.
#[derive(Clone)]
pub struct SandboxStateStoreExt(pub std::sync::Arc<dyn SandboxStateStore>);

/// One hosted implementation normally owns logical identity, incarnation
/// state, and checkpoint lineage in the same database.
pub trait SandboxPersistenceStore:
    SandboxStateStore + crate::sandbox_checkpoint::SandboxCheckpointStore
{
}

impl<T> SandboxPersistenceStore for T where
    T: SandboxStateStore + crate::sandbox_checkpoint::SandboxCheckpointStore
{
}

/// Load managed environment state from durable hosted storage, adopting a
/// legacy session-secret record on first access when necessary.
pub async fn load_session_sandbox_state(
    context: &ToolContext,
) -> Result<Option<SessionSandboxState>, ToolExecutionResult> {
    if let Some(store) = context.extensions.get::<SandboxStateStoreExt>() {
        if let Some(current) = store
            .0
            .load_current_state(context.session_id)
            .await
            .map_err(ToolExecutionResult::internal_error)?
        {
            return Ok(Some(current));
        }

        if let Some(legacy) = load_legacy_session_sandbox_state(context).await? {
            let sandbox = store
                .0
                .save_state(context.session_id, &legacy, None)
                .await
                .map_err(ToolExecutionResult::internal_error)?;
            if let Some(storage) = context.storage_store.as_ref()
                && let Err(error) = storage
                    .delete_secret(context.session_id, SESSION_SANDBOX_SECRET_NAME)
                    .await
            {
                tracing::warn!(%error, "adopted sandbox state but could not remove legacy secret");
            }
            return Ok(Some(SessionSandboxState {
                sandbox: Some(sandbox),
                ..legacy
            }));
        }
    }

    load_legacy_session_sandbox_state(context).await
}

pub(crate) async fn load_state_for_provider(
    context: &ToolContext,
    provider: &str,
) -> Result<Option<SessionSandboxState>, ToolExecutionResult> {
    if let Some(store) = context.extensions.get::<SandboxStateStoreExt>()
        && let Some(state) = store
            .0
            .load_state(context.session_id, provider)
            .await
            .map_err(ToolExecutionResult::internal_error)?
    {
        return Ok(Some(state));
    }
    load_session_sandbox_state(context).await
}

async fn load_legacy_session_sandbox_state(
    context: &ToolContext,
) -> Result<Option<SessionSandboxState>, ToolExecutionResult> {
    let Some(storage) = context.storage_store.as_ref() else {
        return Ok(None);
    };
    let Some(raw) = storage
        .get_secret(context.session_id, SESSION_SANDBOX_SECRET_NAME)
        .await
        .map_err(ToolExecutionResult::internal_error)?
    else {
        return Ok(None);
    };
    serde_json::from_str(&raw).map(Some).map_err(|error| {
        ToolExecutionResult::internal_error_msg(format!(
            "Corrupt legacy session sandbox state: {error}"
        ))
    })
}

/// Persist managed environment state. Hosted PostgreSQL contexts use
/// first-class logical/instance rows; portable hosts retain the legacy secret
/// fallback until they install a [`SandboxStateStore`].
pub async fn save_session_sandbox_state(
    context: &ToolContext,
    state: &mut SessionSandboxState,
) -> Result<(), ToolExecutionResult> {
    validate_sandbox_state(state).map_err(ToolExecutionResult::internal_error)?;
    if let Some(store) = context.extensions.get::<SandboxStateStoreExt>() {
        let sandbox = store
            .0
            .save_state(context.session_id, state, state.sandbox.as_ref())
            .await
            .map_err(ToolExecutionResult::internal_error)?;
        state.sandbox = Some(sandbox);
        return Ok(());
    }

    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;
    let raw = serde_json::to_string(state).map_err(|error| {
        ToolExecutionResult::internal_error_msg(format!(
            "Failed to encode session sandbox state: {error}"
        ))
    })?;
    storage
        .set_secret(context.session_id, SESSION_SANDBOX_SECRET_NAME, &raw)
        .await
        .map_err(ToolExecutionResult::internal_error)
}

/// Delete managed environment state after its provider resource is released.
pub async fn delete_session_sandbox_state(
    context: &ToolContext,
    state: &SessionSandboxState,
) -> Result<(), ToolExecutionResult> {
    if let Some(store) = context.extensions.get::<SandboxStateStoreExt>() {
        store
            .0
            .delete_state(context.session_id, &state.provider, state.sandbox.as_ref())
            .await
            .map_err(ToolExecutionResult::internal_error)?;
    }
    if let Some(storage) = context.storage_store.as_ref() {
        storage
            .delete_secret(context.session_id, SESSION_SANDBOX_SECRET_NAME)
            .await
            .map_err(ToolExecutionResult::internal_error)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use async_trait::async_trait;
    use chrono::Utc;
    use everruns_core::session_services::{KeyInfo, SecretInfo, SessionStorageStore};
    use serde_json::json;

    use super::*;
    use crate::session_sandbox::{SessionSandboxInstance, SessionSandboxStatus};

    #[derive(Default)]
    struct MemoryStateStore {
        state: Mutex<Option<SessionSandboxState>>,
    }

    #[async_trait]
    impl SandboxStateStore for MemoryStateStore {
        async fn load_current_state(
            &self,
            _session_id: SessionId,
        ) -> Result<Option<SessionSandboxState>, SandboxStateError> {
            Ok(self.state.lock().unwrap().clone())
        }

        async fn load_state(
            &self,
            _session_id: SessionId,
            provider: &str,
        ) -> Result<Option<SessionSandboxState>, SandboxStateError> {
            Ok(self
                .state
                .lock()
                .unwrap()
                .clone()
                .filter(|state| state.provider == provider))
        }

        async fn save_state(
            &self,
            _session_id: SessionId,
            state: &SessionSandboxState,
            _expected: Option<&SandboxRef>,
        ) -> Result<SandboxRef, SandboxStateError> {
            let sandbox = SandboxRef {
                id: uuid::Uuid::nil(),
                generation: 1,
            };
            let mut persisted = state.clone();
            persisted.sandbox = Some(sandbox.clone());
            *self.state.lock().unwrap() = Some(persisted);
            Ok(sandbox)
        }

        async fn delete_state(
            &self,
            _session_id: SessionId,
            provider: &str,
            _expected: Option<&SandboxRef>,
        ) -> Result<bool, SandboxStateError> {
            let mut state = self.state.lock().unwrap();
            let matches = state
                .as_ref()
                .is_some_and(|state| state.provider == provider);
            if matches {
                *state = None;
            }
            Ok(matches)
        }
    }

    #[derive(Clone, Default)]
    struct MemorySecrets(Arc<Mutex<HashMap<String, String>>>);

    #[async_trait]
    impl SessionStorageStore for MemorySecrets {
        async fn set_value(
            &self,
            _session_id: SessionId,
            _key: &str,
            _value: &str,
        ) -> everruns_contracts::error::Result<()> {
            unreachable!()
        }

        async fn get_value(
            &self,
            _session_id: SessionId,
            _key: &str,
        ) -> everruns_contracts::error::Result<Option<String>> {
            unreachable!()
        }

        async fn delete_value(
            &self,
            _session_id: SessionId,
            _key: &str,
        ) -> everruns_contracts::error::Result<bool> {
            unreachable!()
        }

        async fn list_keys(
            &self,
            _session_id: SessionId,
        ) -> everruns_contracts::error::Result<Vec<KeyInfo>> {
            unreachable!()
        }

        async fn set_secret(
            &self,
            _session_id: SessionId,
            name: &str,
            value: &str,
        ) -> everruns_contracts::error::Result<()> {
            self.0
                .lock()
                .unwrap()
                .insert(name.to_string(), value.to_string());
            Ok(())
        }

        async fn get_secret(
            &self,
            _session_id: SessionId,
            name: &str,
        ) -> everruns_contracts::error::Result<Option<String>> {
            Ok(self.0.lock().unwrap().get(name).cloned())
        }

        async fn delete_secret(
            &self,
            _session_id: SessionId,
            name: &str,
        ) -> everruns_contracts::error::Result<bool> {
            Ok(self.0.lock().unwrap().remove(name).is_some())
        }

        async fn list_secrets(
            &self,
            _session_id: SessionId,
        ) -> everruns_contracts::error::Result<Vec<SecretInfo>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn hosted_state_lazily_adopts_and_removes_legacy_secret() {
        let storage = Arc::new(MemorySecrets::default());
        let session_id = SessionId::new();
        let legacy_context = ToolContext::with_storage_store(session_id, storage.clone());
        let now = Utc::now().to_rfc3339();
        let mut state = SessionSandboxState {
            sandbox: None,
            provider: "daytona".to_string(),
            status: SessionSandboxStatus::Running,
            instance: SessionSandboxInstance {
                external_id: "sb_legacy".to_string(),
                display_name: None,
                workspace_path: Some("/workspace".to_string()),
                provider_state: json!({}),
                metadata: json!({}),
            },
            init_completed_at: None,
            last_init_error: None,
            created_at: now.clone(),
            updated_at: now,
        };
        save_session_sandbox_state(&legacy_context, &mut state)
            .await
            .unwrap();

        let durable = Arc::new(MemoryStateStore::default());
        let hosted_context =
            legacy_context.with_extension(Arc::new(SandboxStateStoreExt(durable.clone())));
        let adopted = load_session_sandbox_state(&hosted_context)
            .await
            .unwrap()
            .expect("legacy state is adopted");

        assert_eq!(adopted.instance.external_id, "sb_legacy");
        assert!(storage.0.lock().unwrap().is_empty());
        assert_eq!(
            durable
                .state
                .lock()
                .unwrap()
                .as_ref()
                .map(|state| state.instance.external_id.as_str()),
            Some("sb_legacy")
        );
    }

    #[test]
    fn sandbox_state_rejects_oversized_provider_fields() {
        let now = Utc::now().to_rfc3339();
        let mut state = SessionSandboxState {
            sandbox: None,
            provider: "daytona".to_string(),
            status: SessionSandboxStatus::Running,
            instance: SessionSandboxInstance {
                external_id: "sb_test".to_string(),
                display_name: None,
                workspace_path: Some("/workspace".to_string()),
                provider_state: json!({"payload": "x".repeat(MAX_SANDBOX_STATE_FIELD_BYTES)}),
                metadata: json!({}),
            },
            init_completed_at: None,
            last_init_error: None,
            created_at: now.clone(),
            updated_at: now,
        };

        assert!(matches!(
            validate_sandbox_state(&state),
            Err(SandboxStateError::StateTooLarge {
                field: "provider_state",
                ..
            })
        ));

        state.instance.provider_state = json!({});
        state.instance.metadata = json!({"payload": "x".repeat(MAX_SANDBOX_STATE_FIELD_BYTES)});
        assert!(matches!(
            validate_sandbox_state(&state),
            Err(SandboxStateError::StateTooLarge {
                field: "metadata",
                ..
            })
        ));
    }
}
