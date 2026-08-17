//! Public local Git-worktree workspace provider.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use everruns_host::{
    RealDiskFileStore, WorkspaceBinding, WorkspaceCheckpoint, WorkspaceDescriptor, WorkspaceDiff,
    WorkspaceError, WorkspaceHeadAccess, WorkspaceHeadDescriptor, WorkspaceHeadId,
    WorkspaceHeadRequest, WorkspaceHeadResource, WorkspaceHeadStatus, WorkspaceProvider,
    WorkspaceProviderId,
};
use everruns_provider::typed_id::WorkspaceId;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use uuid::Uuid;

const PROVIDER_ID: &str = "everruns.local-git-worktree.v1";
const PAYLOAD: &[u8] = b"local-git-worktree-v1";

/// A Git provider whose writable heads are explicit local worktrees.
///
/// Dropping this value, a Workspace, a head, a Session, or an Agent performs no
/// cleanup. [`WorkspaceHead::destroy`](everruns_host::WorkspaceHead::destroy)
/// explicitly removes only the worktree; its Git branch remains durable.
/// Archive prevents subsequent reopen but does not revoke filesystem handles
/// already held by a running session.
#[derive(Clone, Debug)]
pub struct LocalGitWorkspaceProvider {
    state_root: PathBuf,
    lifecycle_guard: Arc<tokio::sync::Mutex<()>>,
}

impl LocalGitWorkspaceProvider {
    /// Open or create provider state under `state_root`.
    pub fn new(state_root: impl Into<PathBuf>) -> Result<Self, WorkspaceError> {
        let state_root = state_root.into();
        if let Ok(metadata) = std::fs::symlink_metadata(&state_root)
            && (metadata.file_type().is_symlink() || !metadata.is_dir())
        {
            return Err(WorkspaceError::InvalidRequest(
                "local Git provider state root must be a real directory".into(),
            ));
        }
        std::fs::create_dir_all(&state_root).map_err(io_error)?;
        ensure_real_directory(&state_root)?;
        for child in ["workspaces", "heads", "worktrees"] {
            let path = state_root.join(child);
            std::fs::create_dir_all(&path).map_err(io_error)?;
            ensure_real_directory(&path)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&state_root, std::fs::Permissions::from_mode(0o700))
                .map_err(io_error)?;
        }
        Ok(Self {
            state_root,
            lifecycle_guard: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    /// Root containing workspace, head, and worktree lifecycle metadata.
    pub fn state_root(&self) -> &Path {
        &self.state_root
    }

    fn workspace_manifest_path(&self, id: WorkspaceId) -> PathBuf {
        self.state_root
            .join("workspaces")
            .join(format!("{id}.json"))
    }

    fn head_manifest_path(&self, id: WorkspaceHeadId) -> PathBuf {
        self.state_root.join("heads").join(format!("{id}.json"))
    }

    fn worktree_path(&self, workspace: WorkspaceId, head: WorkspaceHeadId) -> PathBuf {
        self.state_root
            .join("worktrees")
            .join(workspace.to_string())
            .join(head.to_string())
    }

    fn read_workspace(&self, id: WorkspaceId) -> Result<WorkspaceManifest, WorkspaceError> {
        let manifest: WorkspaceManifest = read_json(&self.workspace_manifest_path(id))?;
        if manifest.id != id {
            return Err(WorkspaceError::BindingMismatch);
        }
        Ok(manifest)
    }

    fn read_head(&self, id: WorkspaceHeadId) -> Result<HeadManifest, WorkspaceError> {
        let manifest: HeadManifest = read_json(&self.head_manifest_path(id))?;
        let expected_worktree = self.worktree_path(manifest.workspace_id, id);
        let expected_branch = format!("everruns/{id}");
        if manifest.id != id
            || manifest.worktree != expected_worktree
            || manifest.branch != expected_branch
        {
            return Err(WorkspaceError::BindingMismatch);
        }
        Ok(manifest)
    }

    fn write_head(&self, manifest: &HeadManifest) -> Result<(), WorkspaceError> {
        write_json(&self.head_manifest_path(manifest.id), manifest)
    }

    fn validate_binding(&self, binding: &WorkspaceBinding) -> Result<(), WorkspaceError> {
        if binding.provider_id != self.id() || binding.payload != PAYLOAD {
            return Err(WorkspaceError::BindingMismatch);
        }
        Ok(())
    }

    fn bound_head(&self, binding: &WorkspaceBinding) -> Result<HeadManifest, WorkspaceError> {
        self.validate_binding(binding)?;
        let head = self.read_head(binding.head_id)?;
        if head.workspace_id != binding.workspace_id || head.access != binding.access {
            return Err(WorkspaceError::BindingMismatch);
        }
        Ok(head)
    }

    async fn git(&self, repository: &Path, args: &[&str]) -> Result<String, WorkspaceError> {
        let output = Command::new("git")
            .arg("-C")
            .arg(repository)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .await
            .map_err(|error| WorkspaceError::ProviderUnavailable(error.to_string()))?;
        if !output.status.success() {
            return Err(WorkspaceError::Provider(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn resource(
        &self,
        workspace: WorkspaceManifest,
        head: HeadManifest,
    ) -> Result<WorkspaceHeadResource, WorkspaceError> {
        if head.workspace_id != workspace.id {
            return Err(WorkspaceError::BindingMismatch);
        }
        if head.destroyed {
            return Err(WorkspaceError::NotFound);
        }
        if head.archived {
            return Err(WorkspaceError::Archived);
        }
        ensure_real_directory(&head.worktree)?;
        let file_system = Arc::new(
            RealDiskFileStore::new(&head.worktree)
                .map_err(|error| WorkspaceError::ProviderUnavailable(error.to_string()))?,
        );
        let mut metadata = BTreeMap::new();
        metadata.insert("revision".into(), head.revision.clone());
        Ok(WorkspaceHeadResource {
            workspace: WorkspaceDescriptor {
                id: workspace.id,
                name: workspace.name,
                metadata: BTreeMap::new(),
            },
            head: WorkspaceHeadDescriptor {
                id: head.id,
                name: head.name,
                base: head.base,
                access: head.access,
                metadata,
            },
            binding: WorkspaceBinding {
                provider_id: self.id(),
                workspace_id: workspace.id,
                head_id: head.id,
                access: head.access,
                payload: PAYLOAD.to_vec(),
            },
            file_system,
        })
    }
}

#[async_trait]
impl WorkspaceProvider for LocalGitWorkspaceProvider {
    fn id(&self) -> WorkspaceProviderId {
        WorkspaceProviderId::new(PROVIDER_ID).expect("static provider id is valid")
    }

    async fn open_workspace(&self, locator: &str) -> Result<WorkspaceDescriptor, WorkspaceError> {
        let _guard = self.lifecycle_guard.lock().await;
        let requested = std::fs::canonicalize(locator).map_err(|_| WorkspaceError::NotFound)?;
        let root = self
            .git(&requested, &["rev-parse", "--show-toplevel"])
            .await?;
        let repository = std::fs::canonicalize(root).map_err(io_error)?;
        let repository_identity = repository.to_str().ok_or_else(|| {
            WorkspaceError::InvalidRequest(
                "local Git repository path must be valid UTF-8 for durable resume".into(),
            )
        })?;
        let id = WorkspaceId::from_uuid(Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            repository_identity.as_bytes(),
        ));
        let name = repository
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("workspace")
            .to_string();
        let manifest = WorkspaceManifest {
            id,
            name: name.clone(),
            repository,
        };
        write_json(&self.workspace_manifest_path(id), &manifest)?;
        Ok(WorkspaceDescriptor {
            id,
            name,
            metadata: BTreeMap::new(),
        })
    }

    async fn open_workspace_from_binding(
        &self,
        binding: &WorkspaceBinding,
    ) -> Result<WorkspaceDescriptor, WorkspaceError> {
        let _guard = self.lifecycle_guard.lock().await;
        self.bound_head(binding)?;
        let workspace = self.read_workspace(binding.workspace_id)?;
        Ok(WorkspaceDescriptor {
            id: workspace.id,
            name: workspace.name,
            metadata: BTreeMap::new(),
        })
    }

    async fn create_head(
        &self,
        workspace: &WorkspaceDescriptor,
        request: WorkspaceHeadRequest,
    ) -> Result<WorkspaceHeadResource, WorkspaceError> {
        let _guard = self.lifecycle_guard.lock().await;
        if request.name.trim().is_empty() || request.name.len() > 256 {
            return Err(WorkspaceError::InvalidRequest(
                "workspace head name must contain 1..=256 characters".into(),
            ));
        }
        let workspace_manifest = self.read_workspace(workspace.id)?;
        let base = request.base.as_deref().unwrap_or("HEAD");
        validate_revision(base)?;
        let head_id = WorkspaceHeadId::new();
        let worktree = self.worktree_path(workspace.id, head_id);
        if let Some(parent) = worktree.parent() {
            std::fs::create_dir_all(parent).map_err(io_error)?;
        }
        let branch = format!("everruns/{head_id}");
        let worktree_arg = worktree.to_string_lossy().into_owned();
        // THREAT[TM-FS-018]: the repository and base revision are trusted
        // application configuration, never model input. Invoke Git directly
        // (no shell), reject option-like revisions, and generate branch/path
        // identities internally.
        self.git(
            &workspace_manifest.repository,
            &["worktree", "add", "-b", &branch, &worktree_arg, base],
        )
        .await?;
        let revision = self.git(&worktree, &["rev-parse", "HEAD"]).await?;
        let manifest = HeadManifest {
            id: head_id,
            workspace_id: workspace.id,
            name: request.name,
            base: Some(base.to_string()),
            access: request.access,
            worktree,
            branch,
            revision,
            archived: false,
            destroyed: false,
        };
        self.write_head(&manifest)?;
        self.resource(workspace_manifest, manifest)
    }

    async fn reopen_head(
        &self,
        binding: &WorkspaceBinding,
    ) -> Result<WorkspaceHeadResource, WorkspaceError> {
        let _guard = self.lifecycle_guard.lock().await;
        let mut head = self.bound_head(binding)?;
        let workspace = self.read_workspace(binding.workspace_id)?;
        if head.destroyed {
            return Err(WorkspaceError::NotFound);
        }
        if head.archived {
            return Err(WorkspaceError::Archived);
        }
        head.revision = self.git(&head.worktree, &["rev-parse", "HEAD"]).await?;
        self.resource(workspace, head)
    }

    async fn checkpoint(
        &self,
        binding: &WorkspaceBinding,
    ) -> Result<WorkspaceCheckpoint, WorkspaceError> {
        let _guard = self.lifecycle_guard.lock().await;
        let head = self.bound_head(binding)?;
        let revision = self.git(&head.worktree, &["rev-parse", "HEAD"]).await?;
        let mut metadata = BTreeMap::new();
        metadata.insert("head_name".into(), head.name);
        Ok(WorkspaceCheckpoint { revision, metadata })
    }

    async fn status(
        &self,
        binding: &WorkspaceBinding,
    ) -> Result<WorkspaceHeadStatus, WorkspaceError> {
        let _guard = self.lifecycle_guard.lock().await;
        let head = self.bound_head(binding)?;
        if head.destroyed {
            return Err(WorkspaceError::NotFound);
        }
        let porcelain = self
            .git(&head.worktree, &["status", "--porcelain=v1"])
            .await?;
        let conflicts = self
            .git(&head.worktree, &["diff", "--name-only", "--diff-filter=U"])
            .await?;
        let mut metadata = BTreeMap::new();
        metadata.insert("branch".into(), head.branch);
        Ok(WorkspaceHeadStatus {
            dirty: !porcelain.is_empty(),
            conflicted: !conflicts.is_empty(),
            archived: head.archived,
            metadata,
        })
    }

    async fn diff(&self, binding: &WorkspaceBinding) -> Result<WorkspaceDiff, WorkspaceError> {
        let status = self.status(binding).await?;
        Ok(WorkspaceDiff {
            changed: status.dirty,
            conflicted: status.conflicted,
            metadata: status.metadata,
        })
    }

    async fn archive(&self, binding: &WorkspaceBinding) -> Result<(), WorkspaceError> {
        let _guard = self.lifecycle_guard.lock().await;
        let mut head = self.bound_head(binding)?;
        head.archived = true;
        self.write_head(&head)
    }

    async fn destroy(&self, binding: &WorkspaceBinding) -> Result<(), WorkspaceError> {
        let _guard = self.lifecycle_guard.lock().await;
        let mut head = self.bound_head(binding)?;
        let workspace = self.read_workspace(binding.workspace_id)?;
        if !head.destroyed && head.worktree.exists() {
            let worktree = head.worktree.to_string_lossy().into_owned();
            self.git(
                &workspace.repository,
                &["worktree", "remove", "--force", &worktree],
            )
            .await?;
        }
        head.destroyed = true;
        self.write_head(&head)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WorkspaceManifest {
    id: WorkspaceId,
    name: String,
    repository: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct HeadManifest {
    id: WorkspaceHeadId,
    workspace_id: WorkspaceId,
    name: String,
    base: Option<String>,
    access: WorkspaceHeadAccess,
    worktree: PathBuf,
    branch: String,
    revision: String,
    archived: bool,
    destroyed: bool,
}

fn validate_revision(value: &str) -> Result<(), WorkspaceError> {
    if value.is_empty() || value.len() > 512 || value.starts_with('-') || value.contains('\0') {
        return Err(WorkspaceError::InvalidRequest(
            "invalid Git base revision".into(),
        ));
    }
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, WorkspaceError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => WorkspaceError::NotFound,
        _ => io_error(error),
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(WorkspaceError::Provider(
            "workspace metadata must be a regular file".into(),
        ));
    }
    let bytes = std::fs::read(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => WorkspaceError::NotFound,
        _ => io_error(error),
    })?;
    if bytes.len() > 1024 * 1024 {
        return Err(WorkspaceError::Provider(
            "workspace metadata is too large".into(),
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| WorkspaceError::Provider(error.to_string()))
}

fn ensure_real_directory(path: &Path) -> Result<(), WorkspaceError> {
    let metadata = std::fs::symlink_metadata(path).map_err(io_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(WorkspaceError::InvalidRequest(
            "local Git provider paths must be real directories".into(),
        ));
    }
    Ok(())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), WorkspaceError> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| WorkspaceError::Provider(error.to_string()))?;
    let temporary = path.with_extension(format!("json.tmp-{}", Uuid::new_v4()));
    std::fs::write(&temporary, bytes).map_err(io_error)?;
    std::fs::rename(&temporary, path).map_err(io_error)
}

fn io_error(error: impl std::fmt::Display) -> WorkspaceError {
    WorkspaceError::ProviderUnavailable(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::process::Command as StdCommand;

    use everruns_core::session_files::SessionFileSystem;
    use everruns_host::{Workspace, WorkspaceError, WorkspaceHeadAccess};
    use everruns_provider::typed_id::SessionId;

    use super::*;

    fn git(repository: &Path, args: &[&str]) -> String {
        let output = StdCommand::new("git")
            .arg("-C")
            .arg(repository)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn repository() -> tempfile::TempDir {
        let repository = tempfile::tempdir().unwrap();
        git(repository.path(), &["init", "--quiet"]);
        git(
            repository.path(),
            &["config", "user.name", "Workspace Test"],
        );
        git(
            repository.path(),
            &["config", "user.email", "workspace@example.test"],
        );
        std::fs::write(repository.path().join("README.md"), "base\n").unwrap();
        git(repository.path(), &["add", "README.md"]);
        git(repository.path(), &["commit", "--quiet", "-m", "base"]);
        repository
    }

    #[cfg(unix)]
    #[test]
    fn provider_rejects_symlinked_state_directories() {
        let state = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), state.path().join("heads")).unwrap();
        assert!(matches!(
            LocalGitWorkspaceProvider::new(state.path()),
            Err(WorkspaceError::InvalidRequest(_))
        ));
    }

    #[tokio::test]
    async fn isolated_heads_fork_reopen_and_destroy_only_explicitly() {
        let repository = repository();
        let state = tempfile::tempdir().unwrap();
        let provider = Arc::new(LocalGitWorkspaceProvider::new(state.path()).unwrap());
        let workspace = Workspace::open(provider.clone(), repository.path().to_string_lossy())
            .await
            .unwrap();
        let first = workspace.head("first").create().await.unwrap();
        let second = workspace.head("second").create().await.unwrap();
        assert_ne!(first.id(), second.id());
        assert_eq!(first.access(), WorkspaceHeadAccess::Isolated);

        let session = SessionId::new();
        first
            .file_system()
            .write_file(session, "/only-first.txt", "one", "text")
            .await
            .unwrap();
        assert!(
            second
                .file_system()
                .read_file(session, "/only-first.txt")
                .await
                .unwrap()
                .is_none()
        );

        let first_path = state
            .path()
            .join("worktrees")
            .join(workspace.id().to_string())
            .join(first.id().to_string());
        drop(first.clone());
        assert!(first_path.exists(), "Drop must never remove a worktree");

        git(&first_path, &["add", "only-first.txt"]);
        git(&first_path, &["commit", "--quiet", "-m", "head change"]);
        let fork = first.fork("fork").await.unwrap();
        assert!(
            fork.file_system()
                .read_file(session, "/only-first.txt")
                .await
                .unwrap()
                .is_some()
        );

        let reopened = workspace.reopen(first.binding()).await.unwrap();
        assert_eq!(reopened.id(), first.id());
        assert!(
            reopened
                .file_system()
                .read_file(session, "/only-first.txt")
                .await
                .unwrap()
                .is_some()
        );

        let branch = first.status().await.unwrap().metadata["branch"].clone();
        first.clone().destroy().await.unwrap();
        assert!(!first_path.exists());
        git(
            repository.path(),
            &["show-ref", "--verify", &format!("refs/heads/{branch}")],
        );
        assert!(matches!(
            workspace.reopen(first.binding()).await,
            Err(WorkspaceError::NotFound)
        ));
    }

    #[tokio::test]
    async fn shared_and_archive_lifecycle_are_explicit() {
        let repository = repository();
        let state = tempfile::tempdir().unwrap();
        let provider = Arc::new(LocalGitWorkspaceProvider::new(state.path()).unwrap());
        let workspace = Workspace::open(provider, repository.path().to_string_lossy())
            .await
            .unwrap();
        let shared = workspace.head("shared").shared().create().await.unwrap();
        assert_eq!(shared.access(), WorkspaceHeadAccess::Shared);
        let path = state
            .path()
            .join("worktrees")
            .join(workspace.id().to_string())
            .join(shared.id().to_string());

        shared.archive().await.unwrap();
        assert!(path.exists(), "archive retains provider contents");
        assert!(shared.status().await.unwrap().archived);
        assert!(matches!(
            workspace.reopen(shared.binding()).await,
            Err(WorkspaceError::Archived)
        ));
    }
}
