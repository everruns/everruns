//! A platform session's files as the Engine's Environment (data plane v2
//! step 4).
//!
//! Decisions:
//! - The session keeps the files it has on the worker's own host: the
//!   platform's session file store, reached through the same adapter a
//!   worker turn uses. Moving a session onto the Engine moves no file.
//! - One workspace per organization and one head per session, both named by
//!   derived ids, so the backend keeps no state: any worker reopens any
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

fn backend_id() -> WorkspaceBackendId {
    WorkspaceBackendId::new(BACKEND_ID).expect("static backend id is valid")
}

fn workspace_id(org: i64) -> WorkspaceId {
    WorkspaceId::from_uuid(Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("everruns:platform:session-files:{org}").as_bytes(),
    ))
}

fn org_of(locator: &str) -> std::result::Result<i64, WorkspaceError> {
    locator.parse().map_err(|_| WorkspaceError::NotFound)
}

/// The platform's session files, one workspace per organization.
pub struct PlatformFilesBackend<A: WorkerAdapters> {
    adapters: A,
}

impl<A: WorkerAdapters> PlatformFilesBackend<A> {
    pub fn new(adapters: A) -> Self {
        Self { adapters }
    }

    fn descriptor(org: i64) -> WorkspaceDescriptor {
        WorkspaceDescriptor {
            id: workspace_id(org),
            name: "session files".into(),
            // Names the organization for `create_head`.
            metadata: BTreeMap::from([("org".to_string(), org.to_string())]),
        }
    }

    fn org(binding: &WorkspaceBinding) -> std::result::Result<i64, WorkspaceError> {
        let org = std::str::from_utf8(&binding.payload)
            .ok()
            .and_then(|payload| payload.parse().ok())
            .ok_or(WorkspaceError::BindingMismatch)?;
        if binding.provider_id != backend_id() || binding.workspace_id != workspace_id(org) {
            return Err(WorkspaceError::BindingMismatch);
        }
        Ok(org)
    }

    fn resource(
        &self,
        binding: WorkspaceBinding,
    ) -> std::result::Result<WorkspaceHeadResource, WorkspaceError> {
        let org = Self::org(&binding)?;
        Ok(WorkspaceHeadResource {
            workspace: Self::descriptor(org),
            head: WorkspaceHeadDescriptor {
                id: binding.head_id,
                name: binding.head_id.to_string(),
                base: None,
                access: binding.access,
                metadata: BTreeMap::new(),
            },
            binding,
            file_system: Arc::new(SessionAdapter::new(self.adapters.clone()).for_org(org)),
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
        Ok(Self::descriptor(org_of(locator)?))
    }

    async fn open_workspace_from_binding(
        &self,
        binding: &WorkspaceBinding,
    ) -> std::result::Result<WorkspaceDescriptor, WorkspaceError> {
        Ok(Self::descriptor(Self::org(binding)?))
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
        let org = workspace
            .metadata
            .get("org")
            .and_then(|org| org.parse().ok())
            .ok_or(WorkspaceError::NotFound)?;
        if workspace.id != workspace_id(org) {
            return Err(WorkspaceError::BindingMismatch);
        }
        self.resource(WorkspaceBinding {
            provider_id: backend_id(),
            workspace_id: workspace_id(org),
            head_id: WorkspaceHeadId::from_uuid(session),
            access: request.access,
            payload: org.to_string().into_bytes(),
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
        Self::org(binding)?;
        Ok(WorkspaceCheckpoint {
            revision: binding.head_id.to_string(),
            metadata: BTreeMap::new(),
        })
    }

    async fn status(
        &self,
        binding: &WorkspaceBinding,
    ) -> std::result::Result<WorkspaceHeadStatus, WorkspaceError> {
        Self::org(binding)?;
        Ok(WorkspaceHeadStatus::default())
    }

    async fn diff(
        &self,
        binding: &WorkspaceBinding,
    ) -> std::result::Result<WorkspaceDiff, WorkspaceError> {
        Self::org(binding)?;
        Ok(WorkspaceDiff::default())
    }

    async fn archive(&self, binding: &WorkspaceBinding) -> std::result::Result<(), WorkspaceError> {
        Self::org(binding)?;
        Err(WorkspaceError::InvalidRequest(
            "platform session files are archived with their session".into(),
        ))
    }

    async fn destroy(&self, binding: &WorkspaceBinding) -> std::result::Result<(), WorkspaceError> {
        Self::org(binding)?;
        Err(WorkspaceError::InvalidRequest(
            "platform session files are never destroyed by the Engine".into(),
        ))
    }
}

/// Opens a session's Environment over [`PlatformFilesBackend`]. The
/// organization of a session is learned when it is offered.
pub struct PlatformFiles<A: WorkerAdapters> {
    backend: Arc<PlatformFilesBackend<A>>,
    orgs: Arc<Mutex<HashMap<SessionId, i64>>>,
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

    /// Record `session`'s organization before it activates.
    pub fn bind_session(&self, session: SessionId, org_id: i64) {
        self.orgs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(session, org_id);
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
        let org = self
            .orgs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&session)
            .copied()
            .ok_or(AgentLoopError::SessionNotFound(session))?;
        let workspace_error = |error: WorkspaceError| AgentLoopError::store(error.to_string());
        let backend: Arc<dyn WorkspaceBackend> = self.backend.clone();
        let head = Workspace::open(backend, org.to_string())
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
