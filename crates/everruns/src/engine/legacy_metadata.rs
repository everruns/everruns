//! Framework session metadata in caller-owned legacy host backends (EVE-1298).
//!
//! Decision: the workspace binding, Harness requirement and cutover marker of a
//! custom-backend session live in that backend bundle's own
//! [`SessionStorageStore`], under the reserved
//! [`FRAMEWORK_SESSION_KV_PREFIX`](everruns_contracts::runtime::FRAMEWORK_SESSION_KV_PREFIX).
//! They used to live in a process-local store owned by the Agent value, so a
//! host that rebuilt its Agent (a restart, or simply a new value over the same
//! backends) lost the binding and provisioned a fresh default workspace for an
//! existing session. The local profile keeps the same facts in its SQLite
//! catalog instead; see `crate::local::LocalSessionStore`.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::runtime::FRAMEWORK_SESSION_KV_PREFIX;
use everruns_contracts::typed_id::SessionId;
use everruns_core::host::{
    EnvironmentBindingError, EnvironmentBindingStore, InMemoryEnvironmentBindingStore,
    WorkspaceBinding,
};
use everruns_core::session_services::SessionStorageStore;
use serde::{Deserialize, Serialize};

use super::HarnessBindingStore;
use crate::hosting::{AgentBuild, CutoverBoundary};

const ENVIRONMENT_BINDING: &str = "environment_binding";
const HARNESS_REQUIRED: &str = "harness_required";
const CUTOVER: &str = "cutover";

fn key(name: &str) -> String {
    format!("{FRAMEWORK_SESSION_KV_PREFIX}{name}")
}

/// Whether a stored key is Framework metadata rather than session data.
pub(crate) fn is_framework_key(key: &str) -> bool {
    key.starts_with(FRAMEWORK_SESSION_KV_PREFIX)
}

/// Progress of moving a legacy session to a coherent session authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CutoverState {
    /// Ownership decided for the authority; the import has not been confirmed.
    CuttingOver,
    /// The authority committed the import at `boundary`.
    CutOver,
}

/// Durable legacy-side ownership decision. Either state freezes the legacy
/// executor for the session: there is no automatic fallback to it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LegacyCutoverMarker {
    pub state: CutoverState,
    pub target: AgentBuild,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary: Option<CutoverBoundary>,
}

/// Where a legacy lane keeps its cutover marker.
#[async_trait]
pub(crate) trait LegacyCutoverStore: Send + Sync {
    async fn load_cutover(&self, session: SessionId) -> Result<Option<LegacyCutoverMarker>>;
    /// `None` returns ownership to the legacy lane.
    async fn store_cutover(
        &self,
        session: SessionId,
        marker: Option<&LegacyCutoverMarker>,
    ) -> Result<()>;
}

/// Metadata over a caller-owned [`SessionStorageStore`].
pub(crate) struct HostSessionMetadata {
    storage: Arc<dyn SessionStorageStore>,
    /// Cross-session head claims within this backend bundle, as before.
    claims: InMemoryEnvironmentBindingStore,
}

impl HostSessionMetadata {
    pub(crate) fn new(storage: Arc<dyn SessionStorageStore>) -> Self {
        Self {
            storage,
            claims: InMemoryEnvironmentBindingStore::default(),
        }
    }

    async fn stored_binding(
        &self,
        session_id: SessionId,
    ) -> std::result::Result<Option<WorkspaceBinding>, EnvironmentBindingError> {
        let Some(raw) = self
            .storage
            .get_value(session_id, &key(ENVIRONMENT_BINDING))
            .await
            .map_err(|_| EnvironmentBindingError::Unavailable)?
        else {
            return Ok(None);
        };
        let binding: WorkspaceBinding =
            serde_json::from_str(&raw).map_err(|_| EnvironmentBindingError::Corrupt)?;
        binding
            .validate()
            .map_err(|_| EnvironmentBindingError::Corrupt)?;
        Ok(Some(binding))
    }
}

#[async_trait]
impl EnvironmentBindingStore for HostSessionMetadata {
    async fn load(
        &self,
        session_id: SessionId,
    ) -> std::result::Result<Option<WorkspaceBinding>, EnvironmentBindingError> {
        if let Some(binding) = self.claims.load(session_id).await? {
            return Ok(Some(binding));
        }
        let binding = self.stored_binding(session_id).await?;
        if let Some(binding) = &binding {
            // Restore this bundle's claim so another session cannot take an
            // isolated head that a restarted session still owns.
            self.claims.bind(session_id, binding).await?;
        }
        Ok(binding)
    }

    async fn bind(
        &self,
        session_id: SessionId,
        binding: &WorkspaceBinding,
    ) -> std::result::Result<(), EnvironmentBindingError> {
        if let Some(recorded) = self.load(session_id).await? {
            return if &recorded == binding {
                Ok(())
            } else {
                Err(EnvironmentBindingError::Conflict)
            };
        }
        let encoded =
            serde_json::to_string(binding).map_err(|_| EnvironmentBindingError::Corrupt)?;
        // The in-process claim is the serialization point: a concurrent bind of
        // a different head to this session conflicts there before persisting.
        self.claims.bind(session_id, binding).await?;
        // A volatile in-memory head dies with its Agent value; persisting it
        // would make a rebuilt Agent fail to reopen a head that no longer
        // exists instead of starting a fresh volatile one, as before.
        if crate::default_workspace::is_volatile_default(binding) {
            return Ok(());
        }
        self.storage
            .set_value(session_id, &key(ENVIRONMENT_BINDING), &encoded)
            .await
            .map_err(|_| EnvironmentBindingError::Unavailable)
    }
}

#[async_trait]
impl HarnessBindingStore for HostSessionMetadata {
    async fn is_required(
        &self,
        session_id: SessionId,
    ) -> std::result::Result<bool, crate::ResumeError> {
        Ok(self
            .storage
            .get_value(session_id, &key(HARNESS_REQUIRED))
            .await
            .map_err(|_| crate::ResumeError::Unavailable)?
            .is_some())
    }

    async fn require(&self, session_id: SessionId) -> std::result::Result<(), crate::HistoryError> {
        self.storage
            .set_value(session_id, &key(HARNESS_REQUIRED), "true")
            .await
            .map_err(|_| crate::HistoryError::Unavailable)
    }
}

#[async_trait]
impl LegacyCutoverStore for HostSessionMetadata {
    async fn load_cutover(&self, session: SessionId) -> Result<Option<LegacyCutoverMarker>> {
        self.storage
            .get_value(session, &key(CUTOVER))
            .await?
            .map(|raw| {
                serde_json::from_str(&raw)
                    .map_err(|_| AgentLoopError::store("legacy cutover marker is corrupt"))
            })
            .transpose()
    }

    async fn store_cutover(
        &self,
        session: SessionId,
        marker: Option<&LegacyCutoverMarker>,
    ) -> Result<()> {
        match marker {
            Some(marker) => {
                let encoded = serde_json::to_string(marker)
                    .map_err(|error| AgentLoopError::store(error.to_string()))?;
                self.storage
                    .set_value(session, &key(CUTOVER), &encoded)
                    .await
            }
            None => self
                .storage
                .delete_value(session, &key(CUTOVER))
                .await
                .map(|_| ()),
        }
    }
}

/// Hosted sessions have no legacy marker: the authority already owns them.
pub(crate) struct AuthorityOwned;

#[async_trait]
impl LegacyCutoverStore for AuthorityOwned {
    async fn load_cutover(&self, _session: SessionId) -> Result<Option<LegacyCutoverMarker>> {
        Ok(None)
    }

    async fn store_cutover(
        &self,
        _session: SessionId,
        _marker: Option<&LegacyCutoverMarker>,
    ) -> Result<()> {
        Err(AgentLoopError::config(
            "a hosted session has no legacy lane to cut over",
        ))
    }
}
