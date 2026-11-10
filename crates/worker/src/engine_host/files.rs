//! A platform session's files as the Engine's Environment (data plane v2
//! step 4).
//!
//! Decisions:
//! - The session keeps the files it has on the worker's own host: the
//!   platform's session file store, reached through the same adapter a
//!   worker turn uses. Moving a session onto the Engine moves no file.
//! - The Engine's workspace is the platform workspace the session's files
//!   live in, under the same id: the Framework's file tools key a session's
//!   files by its workspace id, and the platform keys them by that workspace
//!   too, so both name the same files. One head per session, named by the
//!   session. The locator and binding name the organization and the
//!   workspace, so the backend keeps no state: any worker reopens any
//!   session's head from its binding alone.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use everruns::hosting::EnvironmentFactory;
use everruns::{
    Environment, Workspace, WorkspaceBackend, WorkspaceBackendId, WorkspaceBinding,
    WorkspaceCheckpoint, WorkspaceDescriptor, WorkspaceDiff, WorkspaceError,
    WorkspaceHeadDescriptor, WorkspaceHeadId, WorkspaceHeadRequest, WorkspaceHeadResource,
    WorkspaceHeadStatus,
};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::{SessionId, WorkspaceId};
use uuid::Uuid;

use crate::worker_adapters::{SessionAdapter, WorkerAdapters};

const BACKEND_ID: &str = "everruns.platform.session-files.v1";

#[expect(
    clippy::expect_used,
    reason = "BACKEND_ID is a constant valid backend id; the test reopening a head proves it"
)]
fn backend_id() -> WorkspaceBackendId {
    WorkspaceBackendId::new(BACKEND_ID).expect("static backend id is valid")
}

/// A platform workspace: its organization and id. Written `{org}/{uuid}` as
/// the locator and as the binding payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformWorkspace {
    pub org: i64,
    pub workspace: Uuid,
}

impl PlatformWorkspace {
    pub fn locator(&self) -> String {
        format!("{}/{}", self.org, self.workspace)
    }

    fn parse(locator: &str) -> Option<Self> {
        let (org, workspace) = locator.split_once('/')?;
        Some(Self {
            org: org.parse().ok()?,
            workspace: workspace.parse().ok()?,
        })
    }

    fn id(&self) -> WorkspaceId {
        WorkspaceId::from_uuid(self.workspace)
    }
}

/// The platform's session files, one Engine workspace per platform workspace.
pub struct PlatformFilesBackend<A: WorkerAdapters> {
    adapters: A,
}

impl<A: WorkerAdapters> PlatformFilesBackend<A> {
    pub fn new(adapters: A) -> Self {
        Self { adapters }
    }

    fn descriptor(workspace: PlatformWorkspace) -> WorkspaceDescriptor {
        WorkspaceDescriptor {
            id: workspace.id(),
            name: "session files".into(),
            // Names the workspace for `create_head`.
            metadata: BTreeMap::from([("platform".to_string(), workspace.locator())]),
        }
    }

    fn workspace(
        binding: &WorkspaceBinding,
    ) -> std::result::Result<PlatformWorkspace, WorkspaceError> {
        let workspace = std::str::from_utf8(&binding.payload)
            .ok()
            .and_then(PlatformWorkspace::parse)
            .ok_or(WorkspaceError::BindingMismatch)?;
        if binding.provider_id != backend_id() || binding.workspace_id != workspace.id() {
            return Err(WorkspaceError::BindingMismatch);
        }
        Ok(workspace)
    }

    fn resource(
        &self,
        binding: WorkspaceBinding,
    ) -> std::result::Result<WorkspaceHeadResource, WorkspaceError> {
        let workspace = Self::workspace(&binding)?;
        Ok(WorkspaceHeadResource {
            workspace: Self::descriptor(workspace),
            head: WorkspaceHeadDescriptor {
                id: binding.head_id,
                name: binding.head_id.to_string(),
                base: None,
                access: binding.access,
                metadata: BTreeMap::new(),
            },
            binding,
            file_system: Arc::new(
                SessionAdapter::new(self.adapters.clone()).for_org(workspace.org),
            ),
        })
    }
}

#[async_trait]
impl<A: WorkerAdapters> WorkspaceBackend for PlatformFilesBackend<A> {
    fn id(&self) -> WorkspaceBackendId {
        backend_id()
    }

    async fn open_workspace(
        &self,
        locator: &str,
    ) -> std::result::Result<WorkspaceDescriptor, WorkspaceError> {
        PlatformWorkspace::parse(locator)
            .map(Self::descriptor)
            .ok_or(WorkspaceError::NotFound)
    }

    async fn open_workspace_from_binding(
        &self,
        binding: &WorkspaceBinding,
    ) -> std::result::Result<WorkspaceDescriptor, WorkspaceError> {
        Ok(Self::descriptor(Self::workspace(binding)?))
    }

    async fn create_head(
        &self,
        workspace: &WorkspaceDescriptor,
        request: WorkspaceHeadRequest,
    ) -> std::result::Result<WorkspaceHeadResource, WorkspaceError> {
        let session: Uuid = request.name.parse().map_err(|_| {
            WorkspaceError::InvalidRequest("a session-files head is named by its session".into())
        })?;
        if request.base.is_some() {
            return Err(WorkspaceError::InvalidRequest(
                "session-files heads have no base".into(),
            ));
        }
        let platform = workspace
            .metadata
            .get("platform")
            .and_then(|locator| PlatformWorkspace::parse(locator))
            .ok_or(WorkspaceError::NotFound)?;
        if workspace.id != platform.id() {
            return Err(WorkspaceError::BindingMismatch);
        }
        self.resource(WorkspaceBinding {
            provider_id: backend_id(),
            workspace_id: platform.id(),
            head_id: WorkspaceHeadId::from_uuid(session),
            access: request.access,
            payload: platform.locator().into_bytes(),
        })
    }

    async fn reopen_head(
        &self,
        binding: &WorkspaceBinding,
    ) -> std::result::Result<WorkspaceHeadResource, WorkspaceError> {
        self.resource(binding.clone())
    }

    async fn checkpoint(
        &self,
        binding: &WorkspaceBinding,
    ) -> std::result::Result<WorkspaceCheckpoint, WorkspaceError> {
        Self::workspace(binding)?;
        Ok(WorkspaceCheckpoint {
            revision: binding.head_id.to_string(),
            metadata: BTreeMap::new(),
        })
    }

    async fn status(
        &self,
        binding: &WorkspaceBinding,
    ) -> std::result::Result<WorkspaceHeadStatus, WorkspaceError> {
        Self::workspace(binding)?;
        Ok(WorkspaceHeadStatus::default())
    }

    async fn diff(
        &self,
        binding: &WorkspaceBinding,
    ) -> std::result::Result<WorkspaceDiff, WorkspaceError> {
        Self::workspace(binding)?;
        Ok(WorkspaceDiff::default())
    }

    async fn archive(&self, binding: &WorkspaceBinding) -> std::result::Result<(), WorkspaceError> {
        Self::workspace(binding)?;
        Err(WorkspaceError::InvalidRequest(
            "platform session files are archived with their session".into(),
        ))
    }

    async fn destroy(&self, binding: &WorkspaceBinding) -> std::result::Result<(), WorkspaceError> {
        Self::workspace(binding)?;
        Err(WorkspaceError::InvalidRequest(
            "platform session files are never destroyed by the Engine".into(),
        ))
    }
}

/// Opens a session's Environment over [`PlatformFilesBackend`]. The
/// workspace of a session is learned when it is offered.
pub struct PlatformFiles<A: WorkerAdapters> {
    backend: Arc<PlatformFilesBackend<A>>,
    orgs: Arc<Mutex<HashMap<SessionId, PlatformWorkspace>>>,
}

impl<A: WorkerAdapters> Clone for PlatformFiles<A> {
    fn clone(&self) -> Self {
        Self {
            backend: self.backend.clone(),
            orgs: self.orgs.clone(),
        }
    }
}

impl<A: WorkerAdapters> PlatformFiles<A> {
    pub fn new(adapters: A) -> Self {
        Self {
            backend: Arc::new(PlatformFilesBackend::new(adapters)),
            orgs: Arc::default(),
        }
    }

    /// Record `session`'s workspace before it activates.
    pub fn bind_session(&self, session: SessionId, workspace: PlatformWorkspace) {
        self.orgs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(session, workspace);
    }

    /// Forget `session` once its activation ended.
    pub fn forget(&self, session: SessionId) {
        self.orgs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&session);
    }
}

#[async_trait]
impl<A: WorkerAdapters> EnvironmentFactory for PlatformFiles<A> {
    async fn open(&self, session: SessionId) -> Result<Environment> {
        let workspace = self
            .orgs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&session)
            .copied()
            .ok_or(AgentLoopError::SessionNotFound(session))?;
        let workspace_error = |error: WorkspaceError| AgentLoopError::store(error.to_string());
        let backend: Arc<dyn WorkspaceBackend> = self.backend.clone();
        let head = Workspace::open(backend, workspace.locator())
            .await
            .map_err(workspace_error)?
            .head(session.uuid().to_string())
            .create()
            .await
            .map_err(workspace_error)?;
        Environment::builder()
            .workspace(head)
            .build()
            .map_err(|error| AgentLoopError::config(error.to_string()))
    }
}
