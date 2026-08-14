//! Deployment-selected session filesystem factories.

use async_trait::async_trait;
use everruns_core::{WorkspaceRootSet, session_files::SessionFileSystem};
use everruns_provider::error::{AgentLoopError, Result};
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

/// Host-supplied values used by platform file-system factories.
///
/// The context is intentionally type-erased so this host contract can accept
/// server-only dependencies such as `StorageBackend` or future object-storage
/// clients without pulling them into `everruns-core`.
#[derive(Clone, Default)]
pub struct SessionFileSystemFactoryContext {
    values: Arc<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl SessionFileSystemFactoryContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with<T: Any + Send + Sync>(mut self, value: Arc<T>) -> Self {
        let values = Arc::make_mut(&mut self.values);
        values.insert(TypeId::of::<T>(), value);
        self
    }

    pub fn get<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.values
            .get(&TypeId::of::<T>())
            .and_then(|value| value.clone().downcast::<T>().ok())
    }

    pub fn with_workspace_roots(self, roots: Arc<WorkspaceRootSet>) -> Self {
        self.with(roots)
    }

    pub fn workspace_roots(&self) -> Option<Arc<WorkspaceRootSet>> {
        self.get::<WorkspaceRootSet>()
    }
}

/// Factory for deployment-selected session filesystem implementations.
#[async_trait]
pub trait SessionFileSystemFactory: Send + Sync {
    /// Human-readable factory name for diagnostics.
    fn name(&self) -> &'static str {
        "SessionFileSystemFactory"
    }

    /// Whether this factory intentionally leaves filesystem selection to the
    /// runtime default.
    fn is_disabled(&self) -> bool {
        false
    }

    /// Resolve a live filesystem from host-provided dependencies.
    async fn create_session_file_system(
        &self,
        context: SessionFileSystemFactoryContext,
    ) -> Result<Arc<dyn SessionFileSystem>>;
}

/// Default factory used when a platform does not configure session files.
#[derive(Debug, Clone, Default)]
pub struct DisabledSessionFileSystemFactory;

#[async_trait]
impl SessionFileSystemFactory for DisabledSessionFileSystemFactory {
    fn name(&self) -> &'static str {
        "DisabledSessionFileSystemFactory"
    }

    fn is_disabled(&self) -> bool {
        true
    }

    async fn create_session_file_system(
        &self,
        _context: SessionFileSystemFactoryContext,
    ) -> Result<Arc<dyn SessionFileSystem>> {
        Err(AgentLoopError::config("session filesystem is disabled"))
    }
}

/// Factory that returns one already-selected session filesystem.
///
/// Workspace providers use this adapter so the runtime consumes the same
/// [`SessionFileSystem`] selected by the head instead of inventing a parallel
/// filesystem abstraction.
#[derive(Clone)]
pub struct FixedSessionFileSystemFactory {
    file_system: Arc<dyn SessionFileSystem>,
}

impl FixedSessionFileSystemFactory {
    pub fn new(file_system: Arc<dyn SessionFileSystem>) -> Self {
        Self { file_system }
    }
}

#[async_trait]
impl SessionFileSystemFactory for FixedSessionFileSystemFactory {
    fn name(&self) -> &'static str {
        "FixedSessionFileSystemFactory"
    }

    async fn create_session_file_system(
        &self,
        _context: SessionFileSystemFactoryContext,
    ) -> Result<Arc<dyn SessionFileSystem>> {
        Ok(self.file_system.clone())
    }
}
