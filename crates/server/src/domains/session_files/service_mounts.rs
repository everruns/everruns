// Mount lifecycle and restoration for existing session workspaces.

use super::*;
use crate::domains::session_files::{memory_mounts, virtual_mount_registry};

impl WorkspaceFileService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            memory_mounts: Arc::new(MemoryMountRouter::new(db.clone())),
            db,
            virtual_registry: Some(Arc::new(virtual_mount_registry::VirtualMountRegistry::new())),
            restored_virtual_mounts: Default::default(),
            quota: QuotaLimits::from_env(),
        }
    }

    /// Virtual docs are process-local. Rebuild them from the session's current
    /// harness on first file access so old chats also work after an upgrade.
    pub(super) async fn restore_virtual_mounts(&self, workspace_id: Uuid) -> Result<()> {
        if self.restored_virtual_mounts.read().contains(&workspace_id) {
            return Ok(());
        }
        let Some(session) = self
            .db
            .get_session_unscoped(SessionId::from_uuid(workspace_id))
            .await?
        else {
            return Ok(());
        };
        if let Some(harness_id) = session.harness_id {
            let harness = crate::domains::harnesses::queries::resolve_effective(
                &self.db,
                session.org_id,
                harness_id,
            )
            .await?;
            if let Some(name) = self.memory_mounts.shared_memory_name(&session).await? {
                memory_mounts::ensure_shared_memory(&self.db, session.org_id, &name).await?;
                self.memory_mounts.evict(&workspace_id);
            }
            let agent_has_platform = match session.agent_id {
                Some(id) => self
                    .db
                    .get_agent_capabilities(id.uuid())
                    .await?
                    .iter()
                    .any(|cap| cap.capability_id == "platform"),
                None => false,
            };
            if agent_has_platform
                || harness.is_some_and(|harness| {
                    harness
                        .capabilities
                        .iter()
                        .any(|cap| cap.capability_id() == "platform")
                })
            {
                use everruns_core::Capability;
                let capability = everruns_capabilities::capabilities::platform::PlatformCapability;
                for mount in capability.mounts() {
                    if let MountSource::Virtual { tree } = mount.source
                        && let Some(registry) = &self.virtual_registry
                    {
                        registry.register(workspace_id, mount.path, tree, "platform".to_string());
                    }
                }
            }
        }
        self.restored_virtual_mounts.write().insert(workspace_id);
        Ok(())
    }

    /// Resolve a path to the Memory that serves it, if any.
    pub(super) async fn route_memory(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Option<(MemoryMount, String)> {
        self.memory_mounts.route(session_id, path).await
    }

    /// Drop a workspace's cached memory mounts (session deleted).
    pub fn evict_memory_mounts(&self, workspace_id: Uuid) {
        self.memory_mounts.evict(&workspace_id);
    }

    pub fn with_virtual_registry(
        mut self,
        registry: Arc<crate::domains::session_files::virtual_mount_registry::VirtualMountRegistry>,
    ) -> Self {
        self.virtual_registry = Some(registry);
        self
    }

    /// Evict virtual mount entries for a session (call on session delete).
    pub fn evict_virtual_mounts(&self, session_id: Uuid) {
        self.restored_virtual_mounts.write().remove(&session_id);
        if let Some(registry) = &self.virtual_registry {
            registry.evict(&session_id);
        }
    }

    /// Get a reference to the virtual mount registry (if configured).
    pub fn virtual_registry(
        &self,
    ) -> Option<&Arc<crate::domains::session_files::virtual_mount_registry::VirtualMountRegistry>>
    {
        self.virtual_registry.as_ref()
    }
}
