// Live routing for server-managed Memory mounts.
//
// A Memory mount used to be a *snapshot*: `apply_capability_mounts` copied the
// Memory's files into `session_files` when the session was created, so a write
// landed in that session's private copy and died with it. Two sessions of the
// same agent, or two chat threads of the same operator, therefore never saw
// each other's notes. `knowledge/runtime-resources/memory.md` always specified
// write-through; this module is it.
//
// Routing is resolved per call against `memory_files`, so a read sees what
// another session wrote a moment ago and a write is durable the moment it
// returns. Nothing is copied.
//
// Only the **server-managed** mounts route here (`/memory/agent`,
// `/memory/user`, `/memory/shared`). They are derivable from the session row
// alone, which is what makes the resolver durable across a restart without a
// mount table: an org-configured `memory` capability mount names an arbitrary
// `mem_` id that only the session's capability config knows, so those keep the
// existing snapshot behavior until that config is persisted somewhere the file
// service can read.
//
// The privacy boundary is unchanged and is load-bearing: `/memory/user` is
// resolved from `resolved_owner_user_id` on the session row, and the file
// service is keyed by *workspace*. For the default one-session workspace the
// two ids are equal; an attached shared workspace has a different key, finds no
// session row, and therefore gets no mounts at all. A shared workspace can
// never route into a private Memory.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use parking_lot::RwLock;
use uuid::Uuid;

use crate::domains::memory::files::{MemoryFileService, MemoryFsError, NewFileInput};
use crate::storage::StorageBackend;
use crate::storage::models::{MemoryFileInfoRow, MemoryRow};

/// Mount point of the memory owned by the session's host agent.
pub const AGENT_MEMORY_MOUNT_PATH: &str = "/memory/agent";
/// Mount point of the session owner's private memory.
pub const USER_MEMORY_MOUNT_PATH: &str = "/memory/user";
/// Mount point of the memory shared by every session of one chat surface.
pub const SHARED_MEMORY_MOUNT_PATH: &str = "/memory/shared";

/// One resolved mount: where it hangs and which Memory backs it.
#[derive(Debug, Clone)]
pub struct MemoryMount {
    pub org_id: i64,
    pub memory_id: Uuid,
    pub mount_path: String,
    /// Source-backed Memories are read-only whatever the mount says.
    pub readonly: bool,
}

impl MemoryMount {
    /// The path *inside* the Memory for a session path under this mount, or
    /// `None` when the path is outside it.
    ///
    /// The mount root itself maps to the Memory root, so `/memory/agent` lists
    /// the Memory's `/`.
    fn inner_path(&self, path: &str) -> Option<String> {
        if path == self.mount_path {
            return Some("/".to_string());
        }
        let rest = path.strip_prefix(&self.mount_path)?;
        rest.starts_with('/').then(|| rest.to_string())
    }

    /// True when `path` is a strict ancestor of this mount's root, so a listing
    /// of `path` must show the mount as a directory entry.
    fn is_ancestor_dir(&self, path: &str) -> bool {
        let prefix = if path == "/" {
            "/".to_string()
        } else {
            format!("{path}/")
        };
        self.mount_path.starts_with(&prefix) && self.mount_path != path
    }

    /// The immediate child of `path` on the way to this mount's root, e.g.
    /// `/memory` when listing `/` and the mount is `/memory/agent`.
    fn child_segment_of(&self, path: &str) -> Option<String> {
        if !self.is_ancestor_dir(path) {
            return None;
        }
        let base = if path == "/" { "" } else { path };
        let rest = self.mount_path.strip_prefix(base)?.trim_start_matches('/');
        let segment = rest.split('/').next()?;
        Some(format!("{base}/{segment}"))
    }
}

/// Resolves and serves the server-managed Memory mounts of one workspace.
pub struct MemoryMountRouter {
    db: Arc<StorageBackend>,
    files: MemoryFileService,
    /// Resolution is three or four queries; a session's mounts do not change
    /// after creation, so resolve once per process per workspace.
    cache: RwLock<HashMap<Uuid, Arc<Vec<MemoryMount>>>>,
}

impl MemoryMountRouter {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            files: MemoryFileService::new(db.clone()),
            db,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Drop a workspace's cached mounts (session deleted, or mounts changed).
    pub fn evict(&self, workspace_id: &Uuid) {
        self.cache.write().remove(workspace_id);
    }

    /// The mounts of one workspace, resolved from its session row.
    pub async fn mounts_for(&self, workspace_id: Uuid) -> Arc<Vec<MemoryMount>> {
        if let Some(cached) = self.cache.read().get(&workspace_id) {
            return cached.clone();
        }
        let resolved = Arc::new(self.resolve(workspace_id).await.unwrap_or_else(|error| {
            // A resolution failure must not take down file access: fall back to
            // "no mounts", which is the pre-write-through behavior.
            tracing::warn!(
                workspace_id = %workspace_id,
                error = %error,
                "failed to resolve memory mounts; serving workspace files only"
            );
            Vec::new()
        }));
        self.cache.write().insert(workspace_id, resolved.clone());
        resolved
    }

    async fn resolve(&self, workspace_id: Uuid) -> Result<Vec<MemoryMount>> {
        let Some(session) = self
            .db
            .get_session_unscoped(everruns_provider::typed_id::SessionId::from_uuid(
                workspace_id,
            ))
            .await?
        else {
            // A shared workspace, or a workspace with no session: no
            // server-managed memory, by design.
            return Ok(Vec::new());
        };

        let mut mounts = Vec::new();
        if let Some(agent_id) = session.agent_id {
            self.push_scoped(
                &mut mounts,
                session.org_id,
                "agent",
                Some(agent_id),
                None,
                AGENT_MEMORY_MOUNT_PATH,
            )
            .await?;
        }
        if let Some(user_id) = session.resolved_owner_user_id {
            self.push_scoped(
                &mut mounts,
                session.org_id,
                "user",
                None,
                Some(user_id),
                USER_MEMORY_MOUNT_PATH,
            )
            .await?;
        }
        if let Some(name) = self.shared_memory_name(&session).await? {
            self.push_named(&mut mounts, session.org_id, &name, SHARED_MEMORY_MOUNT_PATH)
                .await?;
        }

        // Longest mount path first so resolution is unambiguous if one ever
        // nests inside another.
        mounts.sort_by_key(|mount| std::cmp::Reverse(mount.mount_path.len()));
        Ok(mounts)
    }

    /// The reserved Memory name a session's harness shares, if any.
    ///
    /// Keyed on the harness rather than on a capability because the file
    /// service must derive this from the session row alone; resolving an
    /// effective capability set would mean walking the harness chain on every
    /// cold read.
    async fn shared_memory_name(
        &self,
        session: &crate::storage::models::SessionRow,
    ) -> Result<Option<String>> {
        let Some(harness_id) = session.harness_id else {
            return Ok(None);
        };
        let Some(harness) = self.db.get_harness(session.org_id, harness_id).await? else {
            return Ok(None);
        };
        Ok(shared_memory_name_for_harness(&harness.name))
    }

    async fn push_scoped(
        &self,
        mounts: &mut Vec<MemoryMount>,
        org_id: i64,
        scope: &str,
        owner_agent_id: Option<everruns_provider::typed_id::AgentId>,
        owner_user_id: Option<Uuid>,
        mount_path: &str,
    ) -> Result<()> {
        let memory = self
            .db
            .get_memory_by_scope_owner(org_id, scope, owner_agent_id, owner_user_id)
            .await?
            .filter(|memory| memory.status == "active");
        if let Some(memory) = memory {
            mounts.push(mount_from_row(&memory, mount_path));
        }
        Ok(())
    }

    async fn push_named(
        &self,
        mounts: &mut Vec<MemoryMount>,
        org_id: i64,
        name: &str,
        mount_path: &str,
    ) -> Result<()> {
        let memory = self
            .db
            .list_memories(org_id, None, false)
            .await?
            .into_iter()
            .find(|memory| memory.name == name && memory.status == "active");
        if let Some(memory) = memory {
            mounts.push(mount_from_row(&memory, mount_path));
        }
        Ok(())
    }

    /// Resolve a session path to the Memory that serves it.
    pub async fn route(&self, workspace_id: Uuid, path: &str) -> Option<(MemoryMount, String)> {
        let mounts = self.mounts_for(workspace_id).await;
        mounts
            .iter()
            .find_map(|mount| mount.inner_path(path).map(|inner| (mount.clone(), inner)))
    }

    /// Mounts whose root sits below `path`, so a listing of `path` shows them.
    pub async fn mounts_below(&self, workspace_id: Uuid, path: &str) -> Vec<String> {
        let mounts = self.mounts_for(workspace_id).await;
        let mut segments: Vec<String> = mounts
            .iter()
            .filter_map(|mount| mount.child_segment_of(path))
            .collect();
        segments.sort();
        segments.dedup();
        segments
    }

    /// Every mount of this workspace, for recursive listings.
    pub async fn all_mounts(&self, workspace_id: Uuid) -> Arc<Vec<MemoryMount>> {
        self.mounts_for(workspace_id).await
    }

    async fn memory_row(&self, mount: &MemoryMount) -> Result<MemoryRow> {
        self.db
            .get_memory_by_id(mount.org_id, mount.memory_id)
            .await?
            .ok_or_else(|| anyhow!("mounted memory no longer exists"))
    }

    pub async fn read_file(&self, mount: &MemoryMount, inner: &str) -> Result<Option<Vec<u8>>> {
        match self.files.read_file(mount.memory_id, inner).await {
            Ok(read) => Ok(Some(read.content)),
            Err(MemoryFsError::NotFound) => Ok(None),
            Err(MemoryFsError::IsDirectory) => Ok(None),
            Err(error) => Err(map_error(error)),
        }
    }

    pub async fn stat(
        &self,
        mount: &MemoryMount,
        inner: &str,
    ) -> Result<Option<MemoryFileInfoRow>> {
        match self.files.stat(mount.memory_id, inner).await {
            Ok(info) => Ok(Some(info)),
            Err(MemoryFsError::NotFound) => Ok(None),
            Err(error) => Err(map_error(error)),
        }
    }

    pub async fn list_directory(
        &self,
        mount: &MemoryMount,
        inner: &str,
    ) -> Result<Vec<MemoryFileInfoRow>> {
        match self.files.list_directory(mount.memory_id, inner).await {
            Ok(entries) => Ok(entries),
            // The mount root always exists as a directory even when the Memory
            // has no files yet; an empty listing is the truthful answer.
            Err(MemoryFsError::NotFound) if inner == "/" => Ok(Vec::new()),
            Err(error) => Err(map_error(error)),
        }
    }

    pub async fn list_all(&self, mount: &MemoryMount) -> Result<Vec<MemoryFileInfoRow>> {
        Ok(self
            .db
            .list_all_memory_files(mount.memory_id)
            .await?
            .into_iter()
            .map(|row| MemoryFileInfoRow {
                id: row.id,
                memory_id: row.memory_id,
                path: row.path,
                is_directory: row.is_directory,
                size_bytes: row.size_bytes,
                content_hash: row.content_hash,
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
            .collect())
    }

    pub async fn write_file(
        &self,
        mount: &MemoryMount,
        inner: &str,
        content: Vec<u8>,
    ) -> Result<()> {
        self.ensure_writable(mount)?;
        let memory = self.memory_row(mount).await?;
        match self
            .files
            .update_file(&memory, inner, content.clone())
            .await
        {
            Ok(_) => Ok(()),
            Err(MemoryFsError::NotFound) => self
                .files
                .create_file(
                    &memory,
                    NewFileInput {
                        path: inner.to_string(),
                        content: Some(content),
                        is_directory: false,
                    },
                )
                .await
                .map(|_| ())
                .map_err(map_error),
            Err(error) => Err(map_error(error)),
        }
    }

    pub async fn create_directory(&self, mount: &MemoryMount, inner: &str) -> Result<()> {
        self.ensure_writable(mount)?;
        if inner == "/" {
            return Ok(());
        }
        let memory = self.memory_row(mount).await?;
        match self
            .files
            .create_file(
                &memory,
                NewFileInput {
                    path: inner.to_string(),
                    content: None,
                    is_directory: true,
                },
            )
            .await
        {
            Ok(_) => Ok(()),
            // Creating a directory that already exists is not an error for the
            // session filesystem's `ensure_directory_exists` callers.
            Err(MemoryFsError::Conflict(_)) => Ok(()),
            Err(error) => Err(map_error(error)),
        }
    }

    pub async fn delete(&self, mount: &MemoryMount, inner: &str, recursive: bool) -> Result<bool> {
        self.ensure_writable(mount)?;
        let memory = self.memory_row(mount).await?;
        match self.files.delete(&memory, inner, recursive).await {
            Ok(count) => Ok(count > 0),
            Err(MemoryFsError::NotFound) => Ok(false),
            Err(error) => Err(map_error(error)),
        }
    }

    fn ensure_writable(&self, mount: &MemoryMount) -> Result<()> {
        if mount.readonly {
            return Err(anyhow!(
                "Cannot modify readonly file under {}",
                mount.mount_path
            ));
        }
        Ok(())
    }
}

/// The reserved Memory name a harness shares across all of its sessions.
///
/// One entry today. A harness that wants shared memory declares it here rather
/// than by configuration, so the name cannot drift between the session service
/// that creates the Memory and the file service that mounts it.
///
// THREAT[TM-TENANT-015]: a harness listed here gets one org-scoped Memory that
// every session of it reads and writes, so a note one member leaves is visible
// to every other member of that org who can open the same surface. That is the
// feature, not a leak, but it is the one place `/memory` is deliberately not
// private: `/memory/agent` and `/memory/user` stay scoped by owner, and
// `/memory/user` additionally goes through the `resolved_owner_user_id` check
// in `queries::verify_session`. Adding a harness here widens that blast radius
// to its whole org, so it is a source-level allowlist rather than
// configuration, and sharing is not reversible once written.
pub fn shared_memory_name_for_harness(harness_name: &str) -> Option<String> {
    match harness_name {
        crate::harnesses::platform_chat_v2::PLATFORM_CHAT_V2_HARNESS_NAME => {
            Some(PLATFORM_CHAT_SHARED_MEMORY_NAME.to_string())
        }
        _ => None,
    }
}

/// Reserved `memories.name` of the Platform Chat v2 shared memory.
///
/// `UNIQUE(org_id, name)` on live rows is what makes a reserved name a
/// sufficient key: no scope, no migration, and no id a built-in harness
/// definition would have to know.
pub const PLATFORM_CHAT_SHARED_MEMORY_NAME: &str = "platform-chat-shared";

fn mount_from_row(memory: &MemoryRow, mount_path: &str) -> MemoryMount {
    MemoryMount {
        org_id: memory.org_id,
        memory_id: memory.id,
        mount_path: mount_path.to_string(),
        readonly: memory.is_readonly,
    }
}

fn map_error(error: MemoryFsError) -> anyhow::Error {
    anyhow!(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mount(path: &str) -> MemoryMount {
        MemoryMount {
            org_id: 1,
            memory_id: Uuid::nil(),
            mount_path: path.to_string(),
            readonly: false,
        }
    }

    #[test]
    fn mount_root_maps_to_memory_root() {
        assert_eq!(
            mount("/memory/agent")
                .inner_path("/memory/agent")
                .as_deref(),
            Some("/")
        );
    }

    #[test]
    fn paths_under_the_mount_keep_their_suffix() {
        let mount = mount("/memory/shared");
        assert_eq!(
            mount.inner_path("/memory/shared/notes/a.md").as_deref(),
            Some("/notes/a.md")
        );
    }

    /// A sibling whose name merely starts with the mount path must not route:
    /// `/memory/userland` is workspace state, not the private memory.
    #[test]
    fn sibling_prefixes_do_not_route() {
        let mount = mount("/memory/user");
        assert!(mount.inner_path("/memory/userland/a.md").is_none());
        assert!(mount.inner_path("/memory/users").is_none());
        assert!(mount.inner_path("/workspace/other").is_none());
    }

    #[test]
    fn listing_an_ancestor_shows_the_mount_on_the_way_down() {
        let mount = mount("/memory/agent");
        assert_eq!(mount.child_segment_of("/").as_deref(), Some("/memory"));
        assert_eq!(
            mount.child_segment_of("/memory").as_deref(),
            Some("/memory/agent")
        );
        // Not an ancestor of itself, and not of an unrelated directory.
        assert!(mount.child_segment_of("/memory/agent").is_none());
        assert!(mount.child_segment_of("/notes").is_none());
    }

    #[test]
    fn only_declared_harnesses_get_shared_memory() {
        assert_eq!(
            shared_memory_name_for_harness("platform-chat-v2").as_deref(),
            Some(PLATFORM_CHAT_SHARED_MEMORY_NAME)
        );
        assert!(shared_memory_name_for_harness("platform-chat").is_none());
        assert!(shared_memory_name_for_harness("generic").is_none());
    }
}
