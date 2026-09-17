#![allow(deprecated)] // Built-in backends emit legacy errors during their deprecation window.
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use everruns_core::session_files::SessionFileSystem;
use everruns_host::{
    Environment, InMemorySessionFileStore, RealDiskFileStore, Workspace, WorkspaceBackend,
    WorkspaceBackendId, WorkspaceBinding, WorkspaceCheckpoint, WorkspaceDescriptor, WorkspaceDiff,
    WorkspaceError, WorkspaceHeadAccess, WorkspaceHeadDescriptor, WorkspaceHeadId,
    WorkspaceHeadRequest, WorkspaceHeadResource, WorkspaceHeadStatus,
};
use everruns_provider::typed_id::{SessionId, WorkspaceId};
use uuid::Uuid;

pub(crate) struct DefaultWorkspace {
    backend: Arc<dyn WorkspaceBackend>,
    locator: &'static str,
    shared: bool,
}

impl DefaultWorkspace {
    pub(crate) fn in_memory() -> Self {
        Self {
            backend: Arc::new(DefaultMemoryWorkspaceBackend::new()),
            locator: "memory",
            shared: false,
        }
    }

    pub(crate) fn directory(root: PathBuf) -> Self {
        Self {
            backend: Arc::new(DefaultDirectoryWorkspaceBackend { root }),
            locator: "directory",
            shared: true,
        }
    }

    pub(crate) fn backend(&self) -> Arc<dyn WorkspaceBackend> {
        self.backend.clone()
    }

    pub(crate) async fn environment(
        &self,
        session_id: SessionId,
    ) -> Result<Environment, WorkspaceError> {
        let workspace = Workspace::open(self.backend.clone(), self.locator).await?;
        let mut head = workspace.head(if self.shared {
            "default".to_string()
        } else {
            session_id.to_string()
        });
        if self.shared {
            head = head.shared();
        }
        Ok(Environment::builder()
            .workspace(head.create().await?)
            .build()?)
    }
}

impl Clone for DefaultWorkspace {
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            locator: self.locator,
            shared: self.shared,
        }
    }
}

const DIRECTORY_BACKEND_ID: &str = "everruns.framework.directory.v1";
const MEMORY_BACKEND_ID: &str = "everruns.framework.memory.v1";

struct DefaultDirectoryWorkspaceBackend {
    root: PathBuf,
}

impl DefaultDirectoryWorkspaceBackend {
    fn backend_id() -> WorkspaceBackendId {
        WorkspaceBackendId::new(DIRECTORY_BACKEND_ID).expect("static backend id is valid")
    }

    fn root_identity(
        &self,
    ) -> Result<(PathBuf, Vec<u8>, WorkspaceId, WorkspaceHeadId), WorkspaceError> {
        std::fs::create_dir_all(&self.root).map_err(|error| {
            WorkspaceError::Provider(format!("default workspace could not be created: {error}"))
        })?;
        let root = std::fs::canonicalize(&self.root).map_err(|error| {
            WorkspaceError::Provider(format!("default workspace is unavailable: {error}"))
        })?;
        let encoded = root
            .to_str()
            .ok_or_else(|| {
                WorkspaceError::InvalidRequest(
                    "default workspace path must be valid UTF-8 for durable resume".into(),
                )
            })?
            .as_bytes()
            .to_vec();
        let workspace_id = WorkspaceId::from_uuid(Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            &[b"everruns:directory:workspace:", encoded.as_slice()].concat(),
        ));
        let head_id = WorkspaceHeadId::from_uuid(Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            &[b"everruns:directory:head:", encoded.as_slice()].concat(),
        ));
        Ok((root, encoded, workspace_id, head_id))
    }

    fn resource(&self, binding: WorkspaceBinding) -> Result<WorkspaceHeadResource, WorkspaceError> {
        let (root, encoded, workspace_id, head_id) = self.root_identity()?;
        if binding.provider_id != Self::backend_id()
            || binding.workspace_id != workspace_id
            || binding.head_id != head_id
            || binding.access != WorkspaceHeadAccess::Shared
            || binding.payload != encoded
        {
            return Err(WorkspaceError::BindingMismatch);
        }
        let name = root
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("workspace")
            .to_string();
        let file_system: Arc<dyn SessionFileSystem> = Arc::new(
            RealDiskFileStore::new(root)
                .map_err(|error| WorkspaceError::Provider(error.to_string()))?,
        );
        Ok(WorkspaceHeadResource {
            workspace: WorkspaceDescriptor {
                id: workspace_id,
                name,
                metadata: BTreeMap::new(),
            },
            head: WorkspaceHeadDescriptor {
                id: head_id,
                name: "default".into(),
                base: None,
                access: WorkspaceHeadAccess::Shared,
                metadata: BTreeMap::new(),
            },
            binding,
            file_system,
        })
    }

    fn binding(&self) -> Result<WorkspaceBinding, WorkspaceError> {
        let (_, payload, workspace_id, head_id) = self.root_identity()?;
        Ok(WorkspaceBinding {
            provider_id: Self::backend_id(),
            workspace_id,
            head_id,
            access: WorkspaceHeadAccess::Shared,
            payload,
        })
    }
}

#[async_trait]
impl WorkspaceBackend for DefaultDirectoryWorkspaceBackend {
    fn id(&self) -> WorkspaceBackendId {
        Self::backend_id()
    }

    async fn open_workspace(&self, locator: &str) -> Result<WorkspaceDescriptor, WorkspaceError> {
        if locator != "directory" {
            return Err(WorkspaceError::NotFound);
        }
        let resource = self.resource(self.binding()?)?;
        Ok(resource.workspace)
    }

    async fn open_workspace_from_binding(
        &self,
        binding: &WorkspaceBinding,
    ) -> Result<WorkspaceDescriptor, WorkspaceError> {
        Ok(self.resource(binding.clone())?.workspace)
    }

    async fn create_head(
        &self,
        workspace: &WorkspaceDescriptor,
        request: WorkspaceHeadRequest,
    ) -> Result<WorkspaceHeadResource, WorkspaceError> {
        let binding = self.binding()?;
        if workspace.id != binding.workspace_id
            || request.access != WorkspaceHeadAccess::Shared
            || request.base.is_some()
        {
            return Err(WorkspaceError::InvalidRequest(
                "the AgentBuilder::workspace shorthand exposes one shared default head".into(),
            ));
        }
        self.resource(binding)
    }

    async fn reopen_head(
        &self,
        binding: &WorkspaceBinding,
    ) -> Result<WorkspaceHeadResource, WorkspaceError> {
        self.resource(binding.clone())
    }

    async fn checkpoint(
        &self,
        binding: &WorkspaceBinding,
    ) -> Result<WorkspaceCheckpoint, WorkspaceError> {
        self.resource(binding.clone())?;
        Ok(WorkspaceCheckpoint {
            revision: binding.head_id.to_string(),
            metadata: BTreeMap::new(),
        })
    }

    async fn status(
        &self,
        binding: &WorkspaceBinding,
    ) -> Result<WorkspaceHeadStatus, WorkspaceError> {
        self.resource(binding.clone())?;
        Ok(WorkspaceHeadStatus::default())
    }

    async fn diff(&self, binding: &WorkspaceBinding) -> Result<WorkspaceDiff, WorkspaceError> {
        self.resource(binding.clone())?;
        Ok(WorkspaceDiff::default())
    }

    async fn archive(&self, binding: &WorkspaceBinding) -> Result<(), WorkspaceError> {
        self.resource(binding.clone())?;
        Err(WorkspaceError::InvalidRequest(
            "the shared shorthand head cannot be archived".into(),
        ))
    }

    async fn destroy(&self, binding: &WorkspaceBinding) -> Result<(), WorkspaceError> {
        self.resource(binding.clone())?;
        Err(WorkspaceError::InvalidRequest(
            "the shared shorthand head cannot destroy its configured directory".into(),
        ))
    }
}

struct DefaultMemoryWorkspaceBackend {
    instance: Uuid,
    file_system: Arc<dyn SessionFileSystem>,
    heads: Mutex<HashSet<WorkspaceHeadId>>,
    archived: Mutex<HashSet<WorkspaceHeadId>>,
}

impl DefaultMemoryWorkspaceBackend {
    fn new() -> Self {
        Self {
            instance: Uuid::new_v4(),
            file_system: Arc::new(InMemorySessionFileStore::new()),
            heads: Mutex::new(HashSet::new()),
            archived: Mutex::new(HashSet::new()),
        }
    }

    fn backend_id() -> WorkspaceBackendId {
        WorkspaceBackendId::new(MEMORY_BACKEND_ID).expect("static backend id is valid")
    }

    fn workspace_id(&self) -> WorkspaceId {
        WorkspaceId::from_uuid(self.instance)
    }

    fn validate(&self, binding: &WorkspaceBinding) -> Result<(), WorkspaceError> {
        if binding.provider_id != Self::backend_id()
            || binding.workspace_id != self.workspace_id()
            || binding.payload != self.instance.as_bytes()
        {
            return Err(WorkspaceError::BindingMismatch);
        }
        if self
            .archived
            .lock()
            .map_err(|_| WorkspaceError::Provider("workspace state is unavailable".into()))?
            .contains(&binding.head_id)
        {
            return Err(WorkspaceError::Archived);
        }
        if !self
            .heads
            .lock()
            .map_err(|_| WorkspaceError::Provider("workspace state is unavailable".into()))?
            .contains(&binding.head_id)
        {
            return Err(WorkspaceError::NotFound);
        }
        Ok(())
    }

    fn resource(&self, binding: WorkspaceBinding) -> Result<WorkspaceHeadResource, WorkspaceError> {
        self.validate(&binding)?;
        Ok(WorkspaceHeadResource {
            workspace: WorkspaceDescriptor {
                id: self.workspace_id(),
                name: "memory".into(),
                metadata: BTreeMap::new(),
            },
            head: WorkspaceHeadDescriptor {
                id: binding.head_id,
                name: binding.head_id.to_string(),
                base: None,
                access: binding.access,
                metadata: BTreeMap::new(),
            },
            binding,
            file_system: self.file_system.clone(),
        })
    }
}

#[async_trait]
impl WorkspaceBackend for DefaultMemoryWorkspaceBackend {
    fn id(&self) -> WorkspaceBackendId {
        Self::backend_id()
    }

    async fn open_workspace(&self, locator: &str) -> Result<WorkspaceDescriptor, WorkspaceError> {
        if locator != "memory" {
            return Err(WorkspaceError::NotFound);
        }
        Ok(WorkspaceDescriptor {
            id: self.workspace_id(),
            name: "memory".into(),
            metadata: BTreeMap::new(),
        })
    }

    async fn open_workspace_from_binding(
        &self,
        binding: &WorkspaceBinding,
    ) -> Result<WorkspaceDescriptor, WorkspaceError> {
        Ok(self.resource(binding.clone())?.workspace)
    }

    async fn create_head(
        &self,
        workspace: &WorkspaceDescriptor,
        request: WorkspaceHeadRequest,
    ) -> Result<WorkspaceHeadResource, WorkspaceError> {
        if workspace.id != self.workspace_id() || request.base.is_some() {
            return Err(WorkspaceError::InvalidRequest(
                "default memory heads cannot select an external base".into(),
            ));
        }
        let head_id = WorkspaceHeadId::new();
        self.heads
            .lock()
            .map_err(|_| WorkspaceError::Provider("workspace state is unavailable".into()))?
            .insert(head_id);
        self.resource(WorkspaceBinding {
            provider_id: Self::backend_id(),
            workspace_id: self.workspace_id(),
            head_id,
            access: request.access,
            payload: self.instance.as_bytes().to_vec(),
        })
    }

    async fn reopen_head(
        &self,
        binding: &WorkspaceBinding,
    ) -> Result<WorkspaceHeadResource, WorkspaceError> {
        self.resource(binding.clone())
    }

    async fn checkpoint(
        &self,
        binding: &WorkspaceBinding,
    ) -> Result<WorkspaceCheckpoint, WorkspaceError> {
        self.validate(binding)?;
        Ok(WorkspaceCheckpoint {
            revision: binding.head_id.to_string(),
            metadata: BTreeMap::new(),
        })
    }

    async fn status(
        &self,
        binding: &WorkspaceBinding,
    ) -> Result<WorkspaceHeadStatus, WorkspaceError> {
        self.validate(binding)?;
        Ok(WorkspaceHeadStatus::default())
    }

    async fn diff(&self, binding: &WorkspaceBinding) -> Result<WorkspaceDiff, WorkspaceError> {
        self.validate(binding)?;
        Ok(WorkspaceDiff::default())
    }

    async fn archive(&self, binding: &WorkspaceBinding) -> Result<(), WorkspaceError> {
        self.validate(binding)?;
        self.archived
            .lock()
            .map_err(|_| WorkspaceError::Provider("workspace state is unavailable".into()))?
            .insert(binding.head_id);
        Ok(())
    }

    async fn destroy(&self, binding: &WorkspaceBinding) -> Result<(), WorkspaceError> {
        if binding.provider_id != Self::backend_id()
            || binding.workspace_id != self.workspace_id()
            || binding.payload != self.instance.as_bytes()
        {
            return Err(WorkspaceError::BindingMismatch);
        }
        if !self
            .heads
            .lock()
            .map_err(|_| WorkspaceError::Provider("workspace state is unavailable".into()))?
            .remove(&binding.head_id)
        {
            return Err(WorkspaceError::NotFound);
        }
        self.archived
            .lock()
            .map_err(|_| WorkspaceError::Provider("workspace state is unavailable".into()))?
            .remove(&binding.head_id);
        Ok(())
    }
}
