use std::sync::Arc;

use async_trait::async_trait;
use everruns_host::{
    InMemorySessionFileStore, WorkspaceBinding, WorkspaceCheckpoint, WorkspaceDescriptor,
    WorkspaceDiff, WorkspaceError, WorkspaceHeadDescriptor, WorkspaceHeadId, WorkspaceHeadRequest,
    WorkspaceHeadResource, WorkspaceHeadStatus, WorkspaceId, WorkspaceBackend,
    WorkspaceBackendId,
};

/// Compile-only proof that a downstream crate can implement the SPI without a
/// registry entry or closed backend discriminator.
pub struct ExternalWorkspaceBackend;

#[async_trait]
impl WorkspaceBackend for ExternalWorkspaceBackend {
    fn id(&self) -> WorkspaceBackendId {
        WorkspaceBackendId::new("example.external-workspace").unwrap()
    }

    async fn open_workspace(
        &self,
        locator: &str,
    ) -> Result<WorkspaceDescriptor, WorkspaceError> {
        Ok(WorkspaceDescriptor {
            id: WorkspaceId::from_seed(7),
            name: locator.to_owned(),
            metadata: Default::default(),
        })
    }

    async fn open_workspace_from_binding(
        &self,
        _binding: &WorkspaceBinding,
    ) -> Result<WorkspaceDescriptor, WorkspaceError> {
        self.open_workspace("reopened").await
    }

    async fn create_head(
        &self,
        workspace: &WorkspaceDescriptor,
        request: WorkspaceHeadRequest,
    ) -> Result<WorkspaceHeadResource, WorkspaceError> {
        let head_id = WorkspaceHeadId::new();
        Ok(WorkspaceHeadResource {
            workspace: workspace.clone(),
            head: WorkspaceHeadDescriptor {
                id: head_id,
                name: request.name,
                base: request.base,
                access: request.access,
                metadata: Default::default(),
            },
            binding: WorkspaceBinding {
                provider_id: self.id(),
                workspace_id: workspace.id,
                head_id,
                access: request.access,
                payload: b"external-v1".to_vec(),
            },
            file_system: Arc::new(InMemorySessionFileStore::new()),
        })
    }

    async fn reopen_head(
        &self,
        _binding: &WorkspaceBinding,
    ) -> Result<WorkspaceHeadResource, WorkspaceError> {
        Err(WorkspaceError::NotFound)
    }

    async fn checkpoint(
        &self,
        _binding: &WorkspaceBinding,
    ) -> Result<WorkspaceCheckpoint, WorkspaceError> {
        Err(WorkspaceError::NotFound)
    }

    async fn status(
        &self,
        _binding: &WorkspaceBinding,
    ) -> Result<WorkspaceHeadStatus, WorkspaceError> {
        Err(WorkspaceError::NotFound)
    }

    async fn diff(
        &self,
        _binding: &WorkspaceBinding,
    ) -> Result<WorkspaceDiff, WorkspaceError> {
        Ok(WorkspaceDiff::default())
    }

    async fn archive(&self, _binding: &WorkspaceBinding) -> Result<(), WorkspaceError> {
        Ok(())
    }

    async fn destroy(&self, _binding: &WorkspaceBinding) -> Result<(), WorkspaceError> {
        Ok(())
    }
}

#[test]
fn backend_id_is_open_string_data() {
    assert_eq!(
        WorkspaceBackend::id(&ExternalWorkspaceBackend).as_str(),
        "example.external-workspace"
    );
}

#[test]
#[allow(deprecated)]
fn deprecated_provider_names_remain_compatible() {
    use everruns_host::{WorkspaceProvider, WorkspaceProviderId};

    let id: WorkspaceProviderId = WorkspaceProvider::id(&ExternalWorkspaceBackend);
    assert_eq!(id.as_str(), "example.external-workspace");
}
