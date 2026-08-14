//! Host-owned workspace, head, and session-environment contracts.
//!
//! A workspace is logical lineage. A head is one reopenable mutable view of
//! that lineage. Physical storage stays provider-owned and is projected into
//! execution through the existing [`SessionFileSystem`] contract.

use std::any::{Any, TypeId};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use everruns_core::session_files::SessionFileSystem;
use everruns_provider::typed_id::{SessionId, WorkspaceId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Stable, provider-defined SPI identifier.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceProviderId(String);

impl WorkspaceProviderId {
    pub fn new(value: impl Into<String>) -> Result<Self, WorkspaceError> {
        let value = value.into();
        if value.trim().is_empty() || value.len() > 128 {
            return Err(WorkspaceError::InvalidRequest(
                "workspace provider id must contain 1..=128 characters".into(),
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WorkspaceProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Stable identity of one mutable workspace head.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceHeadId(Uuid);

impl WorkspaceHeadId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub const fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub const fn uuid(self) -> Uuid {
        self.0
    }
}

impl Default for WorkspaceHeadId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for WorkspaceHeadId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

/// Provider-owned data sufficient to reopen the exact recorded head.
///
/// Callers persist this value opaquely. Providers must not place credentials
/// in `payload`; local persistence intentionally stores it as plain data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceBinding {
    pub provider_id: WorkspaceProviderId,
    pub workspace_id: WorkspaceId,
    pub head_id: WorkspaceHeadId,
    pub access: WorkspaceHeadAccess,
    #[serde(default)]
    pub payload: Vec<u8>,
}

impl WorkspaceBinding {
    /// Maximum provider payload accepted by Framework persistence.
    pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024;

    pub fn validate(&self) -> Result<(), WorkspaceError> {
        if self.payload.len() > Self::MAX_PAYLOAD_BYTES {
            return Err(WorkspaceError::InvalidRequest(
                "workspace binding payload is too large".into(),
            ));
        }
        Ok(())
    }
}

/// Whether a head is intended for one session or intentionally shared.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceHeadAccess {
    #[default]
    Isolated,
    Shared,
}

/// Provider-neutral identity and base metadata for a logical workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceDescriptor {
    pub id: WorkspaceId,
    pub name: String,
    pub metadata: BTreeMap<String, String>,
}

/// Provider-neutral identity and base metadata for a head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceHeadDescriptor {
    pub id: WorkspaceHeadId,
    pub name: String,
    pub base: Option<String>,
    pub access: WorkspaceHeadAccess,
    pub metadata: BTreeMap<String, String>,
}

/// A provider-produced head resource before the Framework attaches lifecycle.
pub struct WorkspaceHeadResource {
    pub workspace: WorkspaceDescriptor,
    pub head: WorkspaceHeadDescriptor,
    pub binding: WorkspaceBinding,
    pub file_system: Arc<dyn SessionFileSystem>,
}

/// Request to create or fork one head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceHeadRequest {
    pub name: String,
    pub base: Option<String>,
    pub access: WorkspaceHeadAccess,
}

/// Provider-neutral checkpoint metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceCheckpoint {
    pub revision: String,
    pub metadata: BTreeMap<String, String>,
}

/// Provider-neutral mutable-head status.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceHeadStatus {
    pub dirty: bool,
    pub conflicted: bool,
    pub archived: bool,
    pub metadata: BTreeMap<String, String>,
}

/// Provider-neutral diff summary for one mutable head.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceDiff {
    pub changed: bool,
    pub conflicted: bool,
    pub metadata: BTreeMap<String, String>,
}

/// Errors exposed by workspace providers and lifecycle operations.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum WorkspaceError {
    #[error("invalid workspace request: {0}")]
    InvalidRequest(String),
    #[error("workspace provider is unavailable: {0}")]
    ProviderUnavailable(String),
    #[error("workspace or head was not found")]
    NotFound,
    #[error("workspace head is archived")]
    Archived,
    #[error("workspace head has a conflicting update")]
    Conflict,
    #[error("workspace binding does not match the requested provider, workspace, or head")]
    BindingMismatch,
    #[error("workspace provider failed: {0}")]
    Provider(String),
}

/// Open service-provider interface for physical workspace implementations.
///
/// Git worktrees are one implementation. Remote filesystems, containers, and
/// object-backed snapshots can implement the same trait without registering a
/// backend enum or exposing physical paths in the universal contract.
#[async_trait]
pub trait WorkspaceProvider: Send + Sync {
    fn id(&self) -> WorkspaceProviderId;

    async fn open_workspace(&self, locator: &str) -> Result<WorkspaceDescriptor, WorkspaceError>;

    /// Reopen a logical workspace using only its recorded opaque binding.
    async fn open_workspace_from_binding(
        &self,
        binding: &WorkspaceBinding,
    ) -> Result<WorkspaceDescriptor, WorkspaceError>;

    async fn create_head(
        &self,
        workspace: &WorkspaceDescriptor,
        request: WorkspaceHeadRequest,
    ) -> Result<WorkspaceHeadResource, WorkspaceError>;

    async fn reopen_head(
        &self,
        binding: &WorkspaceBinding,
    ) -> Result<WorkspaceHeadResource, WorkspaceError>;

    async fn checkpoint(
        &self,
        binding: &WorkspaceBinding,
    ) -> Result<WorkspaceCheckpoint, WorkspaceError>;

    async fn status(
        &self,
        binding: &WorkspaceBinding,
    ) -> Result<WorkspaceHeadStatus, WorkspaceError>;

    async fn diff(&self, binding: &WorkspaceBinding) -> Result<WorkspaceDiff, WorkspaceError>;

    /// Archive a head while retaining its provider-owned contents.
    async fn archive(&self, binding: &WorkspaceBinding) -> Result<(), WorkspaceError>;

    /// Explicitly destroy provider-owned head storage.
    ///
    /// Providers must never call this from `Drop`. Provider-specific durable
    /// lineage such as a Git branch is retained unless the provider documents
    /// a separate, explicit deletion operation.
    async fn destroy(&self, binding: &WorkspaceBinding) -> Result<(), WorkspaceError>;
}

/// One logical workspace opened through a provider.
#[derive(Clone)]
pub struct Workspace {
    provider: Arc<dyn WorkspaceProvider>,
    descriptor: WorkspaceDescriptor,
}

impl Workspace {
    pub fn from_descriptor(
        provider: Arc<dyn WorkspaceProvider>,
        descriptor: WorkspaceDescriptor,
    ) -> Self {
        Self {
            provider,
            descriptor,
        }
    }

    pub async fn open(
        provider: Arc<dyn WorkspaceProvider>,
        locator: impl AsRef<str>,
    ) -> Result<Self, WorkspaceError> {
        let descriptor = provider.open_workspace(locator.as_ref()).await?;
        Ok(Self {
            provider,
            descriptor,
        })
    }

    pub fn id(&self) -> WorkspaceId {
        self.descriptor.id
    }

    pub fn name(&self) -> &str {
        &self.descriptor.name
    }

    pub fn metadata(&self) -> &BTreeMap<String, String> {
        &self.descriptor.metadata
    }

    pub fn head(&self, name: impl Into<String>) -> WorkspaceHeadBuilder {
        WorkspaceHeadBuilder {
            workspace: self.clone(),
            name: name.into(),
            base: None,
            access: WorkspaceHeadAccess::Isolated,
        }
    }

    pub async fn reopen(
        &self,
        binding: &WorkspaceBinding,
    ) -> Result<WorkspaceHead, WorkspaceError> {
        if binding.provider_id != self.provider.id() || binding.workspace_id != self.id() {
            return Err(WorkspaceError::BindingMismatch);
        }
        let resource = self.provider.reopen_head(binding).await?;
        self.attach(resource, Some(binding))
    }

    fn attach(
        &self,
        resource: WorkspaceHeadResource,
        expected: Option<&WorkspaceBinding>,
    ) -> Result<WorkspaceHead, WorkspaceError> {
        resource.binding.validate()?;
        if resource.workspace.id != self.id()
            || resource.binding.provider_id != self.provider.id()
            || resource.binding.workspace_id != self.id()
            || resource.binding.head_id != resource.head.id
            || resource.binding.access != resource.head.access
            || expected.is_some_and(|expected| expected != &resource.binding)
        {
            return Err(WorkspaceError::BindingMismatch);
        }
        Ok(WorkspaceHead {
            provider: self.provider.clone(),
            workspace: resource.workspace,
            descriptor: resource.head,
            binding: resource.binding,
            file_system: resource.file_system,
        })
    }
}

impl fmt::Debug for Workspace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Workspace")
            .field("provider", &self.provider.id())
            .field("descriptor", &self.descriptor)
            .finish()
    }
}

/// Builder for explicit isolated or shared heads.
pub struct WorkspaceHeadBuilder {
    workspace: Workspace,
    name: String,
    base: Option<String>,
    access: WorkspaceHeadAccess,
}

impl WorkspaceHeadBuilder {
    pub fn from_revision(mut self, revision: impl Into<String>) -> Self {
        self.base = Some(revision.into());
        self
    }

    /// Opt into concurrent sessions addressing the same mutable head.
    pub fn shared(mut self) -> Self {
        self.access = WorkspaceHeadAccess::Shared;
        self
    }

    pub async fn create(self) -> Result<WorkspaceHead, WorkspaceError> {
        if self.name.trim().is_empty() || self.name.len() > 256 {
            return Err(WorkspaceError::InvalidRequest(
                "workspace head name must contain 1..=256 characters".into(),
            ));
        }
        let resource = self
            .workspace
            .provider
            .create_head(
                &self.workspace.descriptor,
                WorkspaceHeadRequest {
                    name: self.name,
                    base: self.base,
                    access: self.access,
                },
            )
            .await?;
        self.workspace.attach(resource, None)
    }
}

/// One stable, reopenable mutable view of a workspace.
#[derive(Clone)]
pub struct WorkspaceHead {
    provider: Arc<dyn WorkspaceProvider>,
    workspace: WorkspaceDescriptor,
    descriptor: WorkspaceHeadDescriptor,
    binding: WorkspaceBinding,
    file_system: Arc<dyn SessionFileSystem>,
}

impl WorkspaceHead {
    /// Provider that owns this head. Applications normally use lifecycle
    /// methods on the head; the facade uses this handle to make typed resume
    /// available for the Agent lifetime.
    pub fn provider(&self) -> Arc<dyn WorkspaceProvider> {
        self.provider.clone()
    }

    pub fn workspace_id(&self) -> WorkspaceId {
        self.workspace.id
    }

    pub fn id(&self) -> WorkspaceHeadId {
        self.descriptor.id
    }

    pub fn name(&self) -> &str {
        &self.descriptor.name
    }

    pub fn base(&self) -> Option<&str> {
        self.descriptor.base.as_deref()
    }

    pub fn access(&self) -> WorkspaceHeadAccess {
        self.descriptor.access
    }

    pub fn binding(&self) -> &WorkspaceBinding {
        &self.binding
    }

    pub fn file_system(&self) -> Arc<dyn SessionFileSystem> {
        self.file_system.clone()
    }

    pub async fn checkpoint(&self) -> Result<WorkspaceCheckpoint, WorkspaceError> {
        self.provider.checkpoint(&self.binding).await
    }

    pub async fn status(&self) -> Result<WorkspaceHeadStatus, WorkspaceError> {
        self.provider.status(&self.binding).await
    }

    pub async fn diff(&self) -> Result<WorkspaceDiff, WorkspaceError> {
        self.provider.diff(&self.binding).await
    }

    pub async fn archive(&self) -> Result<(), WorkspaceError> {
        self.provider.archive(&self.binding).await
    }

    pub async fn destroy(self) -> Result<(), WorkspaceError> {
        self.provider.destroy(&self.binding).await
    }

    /// Create a new isolated head from this head's current checkpoint.
    pub async fn fork(&self, name: impl Into<String>) -> Result<WorkspaceHead, WorkspaceError> {
        let checkpoint = self.checkpoint().await?;
        Workspace {
            provider: self.provider.clone(),
            descriptor: self.workspace.clone(),
        }
        .head(name)
        .from_revision(checkpoint.revision)
        .create()
        .await
    }
}

impl fmt::Debug for WorkspaceHead {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkspaceHead")
            .field("provider", &self.provider.id())
            .field("workspace", &self.workspace)
            .field("descriptor", &self.descriptor)
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

/// Session execution resources. Workspace is first; future compute/network
/// resources attach through the open type-keyed extension seam.
#[derive(Clone)]
pub struct Environment {
    head: WorkspaceHead,
    extensions: Arc<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl Environment {
    pub fn builder() -> EnvironmentBuilder {
        EnvironmentBuilder::default()
    }

    pub fn workspace_head(&self) -> &WorkspaceHead {
        &self.head
    }

    pub fn extension<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.extensions
            .get(&TypeId::of::<T>())
            .and_then(|value| value.clone().downcast().ok())
    }
}

impl fmt::Debug for Environment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Environment")
            .field("head", &self.head)
            .field("extension_count", &self.extensions.len())
            .finish()
    }
}

#[derive(Default)]
pub struct EnvironmentBuilder {
    head: Option<WorkspaceHead>,
    extensions: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl EnvironmentBuilder {
    pub fn workspace(mut self, head: WorkspaceHead) -> Self {
        self.head = Some(head);
        self
    }

    pub fn extension<T: Any + Send + Sync>(mut self, value: Arc<T>) -> Self {
        self.extensions.insert(TypeId::of::<T>(), value);
        self
    }

    /// Attach a typed resource constructed from the selected head.
    ///
    /// Compute providers use this form when their process, container, or
    /// remote mount must address the exact same head as Framework file tools.
    /// Call [`workspace`](Self::workspace) first.
    pub fn workspace_extension<T: Any + Send + Sync>(
        mut self,
        create: impl FnOnce(&WorkspaceHead) -> Arc<T>,
    ) -> Result<Self, WorkspaceError> {
        let head = self.head.as_ref().ok_or_else(|| {
            WorkspaceError::InvalidRequest(
                "workspace must be selected before a workspace extension".into(),
            )
        })?;
        self.extensions.insert(TypeId::of::<T>(), create(head));
        Ok(self)
    }

    pub fn build(self) -> Result<Environment, WorkspaceError> {
        Ok(Environment {
            head: self.head.ok_or_else(|| {
                WorkspaceError::InvalidRequest("environment requires a workspace head".into())
            })?,
            extensions: Arc::new(self.extensions),
        })
    }
}

/// Durable compare-and-set store for a session's opaque environment binding.
#[async_trait]
pub trait EnvironmentBindingStore: Send + Sync {
    async fn load(
        &self,
        session_id: SessionId,
    ) -> Result<Option<WorkspaceBinding>, EnvironmentBindingError>;

    async fn bind(
        &self,
        session_id: SessionId,
        binding: &WorkspaceBinding,
    ) -> Result<(), EnvironmentBindingError>;
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum EnvironmentBindingError {
    #[error("session is already bound to a different workspace head")]
    Conflict,
    #[error("environment binding store is unavailable")]
    Unavailable,
    #[error("persisted environment binding is corrupt")]
    Corrupt,
}

/// Process-local binding store used by the default embedded Agent lifecycle.
#[derive(Default)]
pub struct InMemoryEnvironmentBindingStore {
    bindings: Mutex<HashMap<SessionId, WorkspaceBinding>>,
}

#[async_trait]
impl EnvironmentBindingStore for InMemoryEnvironmentBindingStore {
    async fn load(
        &self,
        session_id: SessionId,
    ) -> Result<Option<WorkspaceBinding>, EnvironmentBindingError> {
        Ok(self
            .bindings
            .lock()
            .map_err(|_| EnvironmentBindingError::Unavailable)?
            .get(&session_id)
            .cloned())
    }

    async fn bind(
        &self,
        session_id: SessionId,
        binding: &WorkspaceBinding,
    ) -> Result<(), EnvironmentBindingError> {
        if binding.payload.len() > WorkspaceBinding::MAX_PAYLOAD_BYTES {
            return Err(EnvironmentBindingError::Corrupt);
        }
        let mut bindings = self
            .bindings
            .lock()
            .map_err(|_| EnvironmentBindingError::Unavailable)?;
        match bindings.get(&session_id) {
            Some(recorded) if recorded != binding => Err(EnvironmentBindingError::Conflict),
            Some(_) => Ok(()),
            None => {
                let incompatible_claim = bindings.iter().any(|(recorded_session, recorded)| {
                    recorded_session != &session_id
                        && recorded.provider_id == binding.provider_id
                        && recorded.workspace_id == binding.workspace_id
                        && recorded.head_id == binding.head_id
                        && (binding.access == WorkspaceHeadAccess::Isolated
                            || recorded.access == WorkspaceHeadAccess::Isolated)
                });
                if incompatible_claim {
                    return Err(EnvironmentBindingError::Conflict);
                }
                bindings.insert(session_id, binding.clone());
                Ok(())
            }
        }
    }
}
