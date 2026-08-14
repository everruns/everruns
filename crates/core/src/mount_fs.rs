// Mount-based virtual filesystem resolver (EVE-660).
//
// `MountFs` is the single resolution seam for the agent's filesystem. It owns:
//
//   * a **mount table** — named mount points, each backed by a
//     `SessionFileSystem`, with a per-mount root in the backend's keyspace, and
//   * a **current working directory** — relative paths resolve against it.
//
// Resolution is uniform and POSIX-shaped: normalize the input against cwd,
// collapse `.`/`..`, then dispatch to the longest matching mount. `/workspace`
// is just a mount point (and the default cwd), not a magic prefix re-implemented
// in every store. Adding `/outputs`, `/.agents/skills`, or volume mounts backed
// by *different* stores later is `with_mount(...)` — the resolver does not change.
//
// Today there is a single workspace backend, so the table holds the root mount
// (`/` → backend, for legacy backend-native paths like `/AGENTS.md`,
// `/outputs/…`) and the `/workspace` view of the same backend. Both resolve to
// the same files; `/workspace` wins by longest-prefix so `/workspace/foo`
// ≡ `/foo`.
//
// See `knowledge/runtime-resources/file-store.md` for the contract and the migration plan.

use async_trait::async_trait;
use std::sync::Arc;

use crate::session_file::{GrepOptions, GrepSearchResult};

use crate::error::{AgentLoopError, Result};
use crate::session_file::{FileInfo, FileStat, GrepMatch, InitialFile, SessionFile};
use crate::session_files::SessionFileSystem;
use crate::typed_id::SessionId;

/// The conventional mount point and default cwd for the workspace. Models
/// trained on cloud-agent layouts address files here; it is a real mount, not a
/// strip-prefix. Same string as [`crate::session_path::WORKSPACE_PREFIX`] (the
/// display alias) — kept as one source of truth.
pub const WORKSPACE_MOUNT: &str = crate::session_path::WORKSPACE_PREFIX;

/// How `MountFs` presents primary-workspace paths to the model, narration,
/// prompts, and persisted output pointers.
///
/// # Why this is a policy seam and not a hardcoded rule
///
/// `/workspace` plays **two independent roles** in `MountFs`, and they must not
/// be conflated:
///
///  1. **Routing / cwd** — the model addresses files at `/workspace/...` and
///     relative paths resolve there. This is a *runtime mechanism*: it is the
///     same for every embedder (models are trained on cloud-agent layouts), so
///     it stays hardcoded as [`WORKSPACE_MOUNT`].
///  2. **Display presentation** — the path string shown to the model, emitted
///     in tool narration, and persisted in output pointers. This is *policy*,
///     and it legitimately differs per embedder. That is what this enum selects.
///
/// # History (do not re-collapse these two roles)
///
///  - PR #2776 made `MountFs` present `/workspace` unconditionally, deleting the
///    old delegation to `backend.display_path(...)`. The motivation was real: a
///    mounted **real-disk server** session leaked the host checkout path
///    (`/private/var/.../checkout/src/lib.rs`) to the model and into persisted,
///    agent-visible output — a host-disclosure issue (threat model TM-FS). In a
///    multi-tenant server the host is infrastructure the model must not see, so
///    `WorkspaceAlias` is the correct, safe **default**.
///  - But #2776 baked that *policy* into the *mechanism*, in shared
///    `everruns-core`. That broke local single-user embedders (e.g. the `yolop`
///    coding CLI, PR #258 "expose real workspace paths"), where the "host" *is*
///    the user's own machine. There, showing `/Users/me/proj/src/lib.rs` is not
///    a disclosure — it is the desired behavior: paths are clickable and match
///    what `bash pwd` prints. Such embedders still need `MountFs` for routing
///    (relative resolution, the default cwd, extra mounts), so they cannot
///    simply drop it; they need presentation to be overridable.
///
/// The resolution: keep the safe alias as the default so no server code changes
/// and #2776's security property is preserved, but expose `BackendNative` so a
/// local embedder can opt back into its backend's real identity. The runtime no
/// longer *hardcodes* presentation — it *defaults* it, and lets the embedder
/// decide. See `knowledge/runtime-resources/file-store.md` (EVE-660, "Display policy").
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DisplayPolicy {
    /// Present primary paths under the stable, host-agnostic `/workspace`
    /// namespace, regardless of what the backend physically is. The default;
    /// required for multi-tenant/server hosts so host paths never reach the
    /// model or persisted output.
    #[default]
    WorkspaceAlias,
    /// Delegate presentation of primary paths to the backend, exposing its
    /// native identity (real host paths for a real-disk store). For local,
    /// single-user embedders where the host is the user's own machine and real
    /// paths are the intended, useful output.
    BackendNative,
}

/// A single entry in the mount table.
#[derive(Clone)]
struct Mount {
    /// Virtual mount point: normalized, absolute, no trailing slash (`/` for
    /// root).
    mount_point: String,
    /// Backend serving this mount.
    backend: Arc<dyn SessionFileSystem>,
    /// Path inside the backend's own keyspace that `mount_point` maps to.
    backend_root: String,
    /// Whether returned backend paths should remain in the backend's canonical
    /// keyspace. The root and `/workspace` mounts preserve the legacy primary
    /// workspace contract (`/src/lib.rs`); named mounts report their mounted
    /// virtual path (`/workspace/roots/backend/src/lib.rs`).
    primary_workspace: bool,
}

#[derive(Clone)]
struct ResolvedMount {
    mount_point: String,
    backend: Arc<dyn SessionFileSystem>,
    backend_root: String,
    backend_path: String,
    primary_workspace: bool,
}

/// Mount-based resolver. Implements `SessionFileSystem`, so it drops into
/// `ToolContext` / `SystemPromptContext` wherever the file store is wired.
pub struct MountFs {
    /// Sorted by mount-point length descending so the first match is the
    /// longest (most specific) mount.
    mounts: Vec<Mount>,
    /// The workspace backend — used as the unreachable resolution fallback.
    primary: Arc<dyn SessionFileSystem>,
    /// Current working directory (a normalized virtual path). Relative inputs
    /// resolve against it; defaults to [`WORKSPACE_MOUNT`]. Fixed at
    /// construction — persistent `cd` across tool calls is not a feature yet.
    cwd: String,
    /// How primary-workspace paths are presented to the model/narration.
    /// Defaults to the host-agnostic alias; embedders opt into backend-native
    /// presentation via [`MountFs::with_backend_display`]. See [`DisplayPolicy`].
    display_policy: DisplayPolicy,
}

impl MountFs {
    /// Build a resolver over a single workspace backend.
    ///
    /// The backend is mounted at both `/` (its native keyspace, for legacy
    /// absolute paths) and `/workspace` (the model-facing view). cwd defaults to
    /// `/workspace`.
    pub fn new(workspace: Arc<dyn SessionFileSystem>) -> Self {
        let mounts = vec![
            Mount {
                mount_point: "/".to_string(),
                backend: workspace.clone(),
                backend_root: "/".to_string(),
                primary_workspace: true,
            },
            Mount {
                mount_point: WORKSPACE_MOUNT.to_string(),
                backend: workspace.clone(),
                backend_root: "/".to_string(),
                primary_workspace: true,
            },
        ];
        let mut fs = Self {
            mounts,
            primary: workspace,
            cwd: WORKSPACE_MOUNT.to_string(),
            display_policy: DisplayPolicy::default(),
        };
        fs.sort_mounts();
        fs
    }

    /// Select how primary-workspace paths are presented. See [`DisplayPolicy`]
    /// for the rationale behind keeping presentation separate from routing.
    pub fn with_display_policy(mut self, policy: DisplayPolicy) -> Self {
        self.display_policy = policy;
        self
    }

    /// Opt into backend-native presentation: primary paths are rendered by the
    /// backend's own `display_path`/`display_root` (real host paths for a
    /// real-disk store), instead of the host-agnostic `/workspace` alias.
    ///
    /// For local, single-user embedders (e.g. a coding CLI) where exposing the
    /// real host path is the intended, useful behavior. Routing is unchanged —
    /// only presentation. Do **not** use this on multi-tenant/server hosts; the
    /// default [`DisplayPolicy::WorkspaceAlias`] keeps host paths out of
    /// model-visible and persisted output (threat model TM-FS, PR #2776).
    pub fn with_backend_display(self) -> Self {
        self.with_display_policy(DisplayPolicy::BackendNative)
    }

    /// Present a resolved primary-workspace backend key to the model, honoring
    /// the configured [`DisplayPolicy`]. Named (non-primary) mounts do not go
    /// through here — they always report their own virtual path.
    fn present_primary_key(&self, canonical_key: &str) -> String {
        match self.display_policy {
            // Host-agnostic: render the backend key literally under /workspace.
            DisplayPolicy::WorkspaceAlias => display_backend_path(WORKSPACE_MOUNT, canonical_key),
            // Backend-native: let the backend expose its own identity (host path).
            DisplayPolicy::BackendNative => self.primary.display_path(canonical_key),
        }
    }

    /// Build a resolver and return it as a trait object.
    pub fn wrap(workspace: Arc<dyn SessionFileSystem>) -> Arc<dyn SessionFileSystem> {
        Arc::new(Self::new(workspace))
    }

    /// Wrap only when `workspace` is not already a [`MountFs`].
    ///
    /// Re-wrapping would collapse nested mount tables (e.g. multi-root
    /// workspaces) into a single primary view and break named-mount display.
    pub fn wrap_if_needed(workspace: Arc<dyn SessionFileSystem>) -> Arc<dyn SessionFileSystem> {
        if workspace.is_mount_resolver() {
            workspace
        } else {
            Self::wrap(workspace)
        }
    }

    /// Register an additional mount (e.g. a read-only skills source or a named
    /// volume) backed by a different store. Longest-prefix wins at resolution.
    pub fn with_mount(
        mut self,
        mount_point: impl Into<String>,
        backend: Arc<dyn SessionFileSystem>,
        backend_root: impl Into<String>,
    ) -> Self {
        self.mounts.push(Mount {
            mount_point: normalize_virtual(&mount_point.into(), "/"),
            backend,
            backend_root: normalize_virtual(&backend_root.into(), "/"),
            primary_workspace: false,
        });
        self.sort_mounts();
        self
    }

    /// The current working directory (normalized virtual path).
    pub fn cwd(&self) -> String {
        self.cwd.clone()
    }

    fn sort_mounts(&mut self) {
        // Longest mount point first, so resolution picks the most specific mount.
        self.mounts
            .sort_by_key(|m| std::cmp::Reverse(m.mount_point.len()));
    }

    /// Resolve any input path to `(backend, backend_path)`.
    ///
    /// Relative inputs are joined to cwd; `.`/`..` are collapsed (clamped at
    /// root); the longest matching mount is selected and the remainder is mapped
    /// into that backend's keyspace.
    fn resolve(&self, input: &str) -> Result<ResolvedMount> {
        reject_additional_root_traversal(input, &self.cwd)?;
        let virtual_path = normalize_virtual(input, &self.cwd());
        for mount in &self.mounts {
            if let Some(rest) = mount_suffix(&mount.mount_point, &virtual_path) {
                return Ok(ResolvedMount {
                    mount_point: mount.mount_point.clone(),
                    backend: mount.backend.clone(),
                    backend_root: mount.backend_root.clone(),
                    backend_path: join_backend_path(&mount.backend_root, &rest),
                    primary_workspace: mount.primary_workspace,
                });
            }
        }
        // The root mount matches every absolute path, so this is unreachable in
        // practice; fall back to the primary backend with the literal path.
        Ok(ResolvedMount {
            mount_point: "/".to_string(),
            backend: self.primary.clone(),
            backend_root: "/".to_string(),
            backend_path: virtual_path,
            primary_workspace: true,
        })
    }

    fn grep_mounts(&self) -> Vec<ResolvedMount> {
        let mut out: Vec<ResolvedMount> = Vec::new();
        for mount in self.mounts.iter().rev() {
            if out.iter().any(|existing| {
                Arc::ptr_eq(&existing.backend, &mount.backend)
                    && existing.backend_root == mount.backend_root
            }) {
                continue;
            }
            out.push(ResolvedMount {
                mount_point: mount.mount_point.clone(),
                backend: mount.backend.clone(),
                backend_root: mount.backend_root.clone(),
                backend_path: mount.backend_root.clone(),
                primary_workspace: mount.primary_workspace,
            });
        }
        out
    }
}

impl ResolvedMount {
    fn map_session_file(&self, mut file: SessionFile) -> SessionFile {
        file.path = self.to_virtual_output_path(&file.path);
        file.name = FileInfo::name_from_path(&file.path);
        file
    }

    fn map_file_info(&self, mut info: FileInfo) -> FileInfo {
        info.path = self.to_virtual_output_path(&info.path);
        info.name = FileInfo::name_from_path(&info.path);
        info
    }

    fn map_file_stat(&self, mut stat: FileStat) -> FileStat {
        stat.path = self.to_virtual_output_path(&stat.path);
        stat.name = FileInfo::name_from_path(&stat.path);
        stat
    }

    fn map_grep_match(&self, mut grep_match: GrepMatch) -> GrepMatch {
        grep_match.path = self.to_virtual_output_path(&grep_match.path);
        grep_match
    }

    fn map_grep_result(&self, mut result: GrepSearchResult) -> GrepSearchResult {
        for grep_match in &mut result.matches {
            grep_match.path = self.to_virtual_output_path(&grep_match.path);
        }
        for block in &mut result.blocks {
            block.path = self.to_virtual_output_path(&block.path);
        }
        result
    }

    fn to_virtual_output_path(&self, backend_path: &str) -> String {
        if self.primary_workspace {
            return normalize_virtual(backend_path, "/");
        }
        let normalized = normalize_virtual(backend_path, "/");
        let rest = mount_suffix(&self.backend_root, &normalized).unwrap_or(normalized);
        join_backend_path(&self.mount_point, &rest)
    }
}

/// Wrap an embedder's file store for a model-facing [`SystemPromptContext`]:
/// pin reads to the session's workspace, then guarantee a `/workspace` mount +
/// cwd — WITHOUT discarding a display policy the embedder already configured.
///
/// This is the single, shared way the reason path
/// ([`crate::runtime_context::assemble_turn_context`]) and the executor path
/// (`everruns_host`'s `load_execution_capabilities`) build their prompt
/// file store. Both used to inline `MountFs::wrap(WorkspaceScopedFileSystem::…)`
/// separately; they drifted from the tool-execution (`act`) path — which wraps
/// with [`MountFs::wrap_if_needed`] — so a local embedder's real host paths
/// reached tool narration but were forced back to `/workspace` in the system
/// prompt. Centralizing here keeps all three paths identical so presentation
/// cannot drift again.
///
/// Why `wrap_if_needed` (not `wrap`): if `file_store` is already a [`MountFs`]
/// (a local embedder that opted into [`DisplayPolicy::BackendNative`] via
/// [`MountFs::with_backend_display`]), re-wrapping would bury it under a fresh
/// default-`WorkspaceAlias` resolver and re-hide the host path — regressing
/// EVE-660 / #258 host-path presentation. `wrap_if_needed` preserves the
/// embedder's resolver (and its policy), and also avoids collapsing multi-root
/// named mounts (see [`MountFs::wrap_if_needed`]).
///
/// Multi-tenant/server stores are unaffected: they are not mount resolvers, so
/// they still get wrapped into the default `/workspace` alias, keeping host
/// paths out of model-visible output (#2776, threat model TM-FS).
pub fn scoped_prompt_file_store(
    file_store: Arc<dyn SessionFileSystem>,
    workspace_id: crate::typed_id::WorkspaceId,
) -> Arc<dyn SessionFileSystem> {
    MountFs::wrap_if_needed(crate::session_files::WorkspaceScopedFileSystem::wrap(
        file_store,
        workspace_id,
    ))
}

/// Normalize an input into an absolute virtual path: join cwd if relative, then
/// collapse `.`/`..` segments (a leading `..` is clamped at root).
fn normalize_virtual(input: &str, cwd: &str) -> String {
    let combined = if input.starts_with('/') {
        input.to_string()
    } else {
        format!("{}/{}", cwd.trim_end_matches('/'), input)
    };
    let mut stack: Vec<&str> = Vec::new();
    for segment in combined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            other => stack.push(other),
        }
    }
    if stack.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", stack.join("/"))
    }
}

fn reject_additional_root_traversal(input: &str, cwd: &str) -> Result<()> {
    let combined = if input.starts_with('/') {
        input.to_string()
    } else {
        format!("{}/{}", cwd.trim_end_matches('/'), input)
    };
    let segments: Vec<&str> = combined
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    for window_start in 0..segments.len().saturating_sub(2) {
        if segments[window_start] == "workspace" && segments[window_start + 1] == "roots" {
            let root_name_idx = window_start + 2;
            if segments[root_name_idx].is_empty() {
                continue;
            }
            if segments
                .iter()
                .skip(root_name_idx + 1)
                .any(|segment| *segment == "..")
            {
                return Err(AgentLoopError::tool(format!(
                    "path traversal rejected: {input}"
                )));
            }
        }
    }
    Ok(())
}

/// If `virtual_path` is at or under `mount_point`, return the suffix as a
/// `/`-rooted remainder (`/` for an exact match). Segment-aware: `/workspacefoo`
/// is not under `/workspace`.
fn mount_suffix(mount_point: &str, virtual_path: &str) -> Option<String> {
    if mount_point == "/" {
        // The root mount owns the whole path.
        return Some(virtual_path.to_string());
    }
    if virtual_path == mount_point {
        return Some("/".to_string());
    }
    virtual_path
        .strip_prefix(mount_point)
        .filter(|rest| rest.starts_with('/'))
        .map(|rest| rest.to_string())
}

/// Join a backend root with a `/`-rooted remainder into a backend keyspace path.
fn join_backend_path(backend_root: &str, rest: &str) -> String {
    if backend_root == "/" {
        return rest.to_string();
    }
    if rest == "/" {
        return backend_root.to_string();
    }
    format!("{backend_root}{rest}")
}

/// Render a canonical backend key literally under that backend's display root.
fn display_backend_path(display_root: &str, path: &str) -> String {
    let normalized = normalize_virtual(path, "/");
    if normalized == "/" {
        display_root.to_string()
    } else if display_root == "/" {
        normalized
    } else {
        format!("{}{normalized}", display_root.trim_end_matches('/'))
    }
}

#[async_trait]
impl SessionFileSystem for MountFs {
    fn display_root(&self) -> String {
        // Routing always defaults cwd to /workspace, but the *displayed* root is
        // policy: the alias for host-agnostic presentation, or the backend's own
        // root (a real host directory) when the embedder opts in. See
        // [`DisplayPolicy`].
        match self.display_policy {
            DisplayPolicy::WorkspaceAlias => WORKSPACE_MOUNT.to_string(),
            DisplayPolicy::BackendNative => self.primary.display_root(),
        }
    }

    fn is_mount_resolver(&self) -> bool {
        true
    }

    fn resolve_path(&self, input: &str) -> String {
        // Resolve the raw input through the mount table, then present the
        // resolved backend key per the display policy. Presenting the resolved
        // backend key (instead of re-stripping the mount) keeps a literal
        // `workspace/…` backend segment distinct from the `/workspace` mount
        // alias, so a displayed path round-trips to the same backend key.
        // Presentation itself (alias vs. backend-native host path) is decided by
        // [`present_primary_key`] — kept out of routing on purpose (#2776/#258).
        let virtual_path = normalize_virtual(input, &self.cwd());
        match self.resolve(&virtual_path) {
            Ok(resolved) if resolved.primary_workspace => {
                self.present_primary_key(&resolved.backend_path)
            }
            _ => virtual_path,
        }
    }

    fn display_path(&self, path: &str) -> String {
        // `path` here is an already-canonical virtual output path (a resolved
        // `file.path`, i.e. a backend key in the primary namespace, or a named
        // mount's virtual path). Normalize at root — NOT cwd — so we treat it as
        // a canonical key and don't inject the `/workspace` cwd alias. Then:
        //  - named mounts: return as-is (already in their mounted namespace),
        //  - primary: present via the display policy so a literal `workspace/…`
        //    backend segment stays distinct from the mount alias and round-trips
        //    (alias mode), or renders as the backend's host path (native mode).
        let virtual_path = normalize_virtual(path, "/");
        match self.resolve(&virtual_path) {
            Ok(resolved) if !resolved.primary_workspace => virtual_path,
            _ => self.present_primary_key(&virtual_path),
        }
    }

    async fn read_file(&self, session_id: SessionId, path: &str) -> Result<Option<SessionFile>> {
        let resolved = self.resolve(path)?;
        Ok(resolved
            .backend
            .read_file(session_id, &resolved.backend_path)
            .await?
            .map(|file| resolved.map_session_file(file)))
    }

    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        let resolved = self.resolve(path)?;
        Ok(resolved.map_session_file(
            resolved
                .backend
                .write_file(session_id, &resolved.backend_path, content, encoding)
                .await?,
        ))
    }

    async fn write_file_if_content_matches(
        &self,
        session_id: SessionId,
        path: &str,
        expected_content: &str,
        expected_encoding: &str,
        content: &str,
        encoding: &str,
    ) -> Result<Option<SessionFile>> {
        let resolved = self.resolve(path)?;
        Ok(resolved
            .backend
            .write_file_if_content_matches(
                session_id,
                &resolved.backend_path,
                expected_content,
                expected_encoding,
                content,
                encoding,
            )
            .await?
            .map(|file| resolved.map_session_file(file)))
    }

    async fn delete_file(
        &self,
        session_id: SessionId,
        path: &str,
        recursive: bool,
    ) -> Result<bool> {
        let resolved = self.resolve(path)?;
        resolved
            .backend
            .delete_file(session_id, &resolved.backend_path, recursive)
            .await
    }

    async fn list_directory(&self, session_id: SessionId, path: &str) -> Result<Vec<FileInfo>> {
        let resolved = self.resolve(path)?;
        Ok(resolved
            .backend
            .list_directory(session_id, &resolved.backend_path)
            .await?
            .into_iter()
            .map(|info| resolved.map_file_info(info))
            .collect())
    }

    async fn stat_file(&self, session_id: SessionId, path: &str) -> Result<Option<FileStat>> {
        let resolved = self.resolve(path)?;
        Ok(resolved
            .backend
            .stat_file(session_id, &resolved.backend_path)
            .await?
            .map(|stat| resolved.map_file_stat(stat)))
    }

    async fn grep_files(
        &self,
        session_id: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        match path_pattern {
            Some(pp) => {
                let matcher = crate::session_path::GrepPathPattern::new(pp)?;
                if matcher.is_glob() && (!pp.starts_with('/') || pp.starts_with(WORKSPACE_MOUNT)) {
                    let mut matches = Vec::new();
                    for resolved in self.grep_mounts() {
                        matches.extend(
                            resolved
                                .backend
                                .grep_files(session_id, pattern, Some(&resolved.backend_path))
                                .await?
                                .into_iter()
                                .map(|grep_match| resolved.map_grep_match(grep_match))
                                .filter(|grep_match| matcher.is_match(&grep_match.path)),
                        );
                    }
                    matches.sort_by(|a, b| {
                        a.path
                            .cmp(&b.path)
                            .then(a.line_number.cmp(&b.line_number))
                            .then(a.line.cmp(&b.line))
                    });
                    return Ok(matches);
                }
                let resolved = self.resolve(pp)?;
                Ok(resolved
                    .backend
                    .grep_files(session_id, pattern, Some(&resolved.backend_path))
                    .await?
                    .into_iter()
                    .map(|grep_match| resolved.map_grep_match(grep_match))
                    .collect())
            }
            None => {
                let mut matches = Vec::new();
                for resolved in self.grep_mounts() {
                    matches.extend(
                        resolved
                            .backend
                            .grep_files(session_id, pattern, Some(&resolved.backend_path))
                            .await?
                            .into_iter()
                            .map(|grep_match| resolved.map_grep_match(grep_match)),
                    );
                }
                matches.sort_by(|a, b| {
                    a.path
                        .cmp(&b.path)
                        .then(a.line_number.cmp(&b.line_number))
                        .then(a.line.cmp(&b.line))
                });
                Ok(matches)
            }
        }
    }

    async fn grep_files_with_options(
        &self,
        session_id: SessionId,
        pattern: &str,
        options: &GrepOptions,
    ) -> Result<GrepSearchResult> {
        if let Some(path_pattern) = options.path_pattern.as_deref()
            && path_pattern.starts_with('/')
            && !path_pattern.starts_with(WORKSPACE_MOUNT)
        {
            let resolved = self.resolve(path_pattern)?;
            let mut backend_options = options.clone();
            backend_options.path_pattern = Some(resolved.backend_path.clone());
            return resolved
                .backend
                .grep_files_with_options(session_id, pattern, &backend_options)
                .await
                .map(|result| resolved.map_grep_result(result));
        }

        let mounts = self.grep_mounts();
        if mounts.len() == 1 {
            let resolved = &mounts[0];
            let mut backend_options = options.clone();
            backend_options.path_pattern = options.path_pattern.as_ref().map(|path| {
                if path.starts_with(WORKSPACE_MOUNT) {
                    path.strip_prefix(WORKSPACE_MOUNT)
                        .unwrap_or(path)
                        .to_string()
                } else {
                    path.clone()
                }
            });
            return resolved
                .backend
                .grep_files_with_options(session_id, pattern, &backend_options)
                .await
                .map(|result| resolved.map_grep_result(result));
        }

        let mut backend_options = options.clone();
        backend_options.offset = 0;
        backend_options.limit = usize::MAX;
        backend_options.max_bytes = usize::MAX;
        let path_matcher = options
            .path_pattern
            .as_deref()
            .map(crate::session_path::GrepPathPattern::new)
            .transpose()?;
        let mut results = Vec::new();
        for resolved in mounts {
            let mut mount_options = backend_options.clone();
            mount_options.path_pattern = Some(resolved.backend_path.clone());
            let result = resolved
                .backend
                .grep_files_with_options(session_id, pattern, &mount_options)
                .await?;
            let mut mapped = resolved.map_grep_result(result);
            if let Some(matcher) = &path_matcher {
                mapped.matches.retain(|item| matcher.is_match(&item.path));
                mapped.blocks.retain(|block| matcher.is_match(&block.path));
            }
            results.push(mapped);
        }
        Ok(crate::session_file::merge_grep_search_results(
            results, options,
        ))
    }

    async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
        let resolved = self.resolve(path)?;
        Ok(resolved.map_file_info(
            resolved
                .backend
                .create_directory(session_id, &resolved.backend_path)
                .await?,
        ))
    }

    async fn seed_initial_file(&self, session_id: SessionId, file: &InitialFile) -> Result<()> {
        let resolved = self.resolve(&file.path)?;
        let seeded = InitialFile {
            path: resolved.backend_path,
            content: file.content.clone(),
            encoding: file.encoding.clone(),
            is_readonly: file.is_readonly,
        };
        resolved
            .backend
            .seed_initial_file(session_id, &seeded)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_path::GrepPathPattern;

    fn sid() -> SessionId {
        SessionId::from_seed(1)
    }

    // A minimal `/`-rooted in-memory backend for resolver tests (kept local to
    // avoid a dependency on everruns-host).
    #[derive(Default)]
    struct FlatStore {
        files: std::sync::Mutex<std::collections::HashMap<String, String>>,
        /// When set, the store presents host-style paths like a real-disk
        /// backend, so `DisplayPolicy::BackendNative` has a host identity to
        /// delegate to. `None` keeps the trait-default `/workspace` alias.
        host_root: Option<String>,
    }

    #[async_trait]
    impl SessionFileSystem for FlatStore {
        fn is_mount_resolver(&self) -> bool {
            false
        }

        fn display_root(&self) -> String {
            match &self.host_root {
                Some(root) => root.clone(),
                None => crate::session_path::WORKSPACE_PREFIX.to_string(),
            }
        }

        fn display_path(&self, path: &str) -> String {
            match &self.host_root {
                Some(root) => {
                    let normalized = normalize_virtual(path, "/");
                    if normalized == "/" {
                        root.clone()
                    } else {
                        format!("{root}{normalized}")
                    }
                }
                None => crate::session_path::to_display_path(path),
            }
        }

        async fn read_file(&self, sid: SessionId, path: &str) -> Result<Option<SessionFile>> {
            let files = self.files.lock().unwrap();
            Ok(files.get(path).map(|content| SessionFile {
                id: uuid::Uuid::nil(),
                session_id: sid.uuid(),
                path: path.to_string(),
                name: path.rsplit('/').next().unwrap_or("").to_string(),
                content: Some(content.clone()),
                encoding: "text".to_string(),
                is_directory: false,
                is_readonly: false,
                size_bytes: content.len() as i64,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }))
        }
        async fn write_file(
            &self,
            sid: SessionId,
            path: &str,
            content: &str,
            encoding: &str,
        ) -> Result<SessionFile> {
            self.files
                .lock()
                .unwrap()
                .insert(path.to_string(), content.to_string());
            Ok(SessionFile {
                id: uuid::Uuid::nil(),
                session_id: sid.uuid(),
                path: path.to_string(),
                name: path.rsplit('/').next().unwrap_or("").to_string(),
                content: Some(content.to_string()),
                encoding: encoding.to_string(),
                is_directory: false,
                is_readonly: false,
                size_bytes: content.len() as i64,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
        }
        async fn delete_file(&self, _: SessionId, path: &str, _: bool) -> Result<bool> {
            Ok(self.files.lock().unwrap().remove(path).is_some())
        }
        async fn list_directory(&self, _: SessionId, _: &str) -> Result<Vec<FileInfo>> {
            Ok(vec![])
        }
        async fn stat_file(&self, _: SessionId, path: &str) -> Result<Option<FileStat>> {
            let files = self.files.lock().unwrap();
            Ok(files.get(path).map(|content| FileStat {
                path: path.to_string(),
                name: path.rsplit('/').next().unwrap_or("").to_string(),
                is_directory: false,
                is_readonly: false,
                size_bytes: content.len() as i64,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }))
        }
        async fn grep_files(
            &self,
            _: SessionId,
            pattern: &str,
            path_pattern: Option<&str>,
        ) -> Result<Vec<GrepMatch>> {
            let path_pattern = path_pattern.map(GrepPathPattern::new).transpose()?;
            let files = self.files.lock().unwrap();
            let mut matches = Vec::new();
            for (path, content) in files.iter() {
                if let Some(filter) = &path_pattern
                    && !filter.is_match(path)
                {
                    continue;
                }
                for (idx, line) in content.lines().enumerate() {
                    if line.contains(pattern) {
                        matches.push(GrepMatch {
                            path: path.clone(),
                            line_number: idx + 1,
                            line: line.to_string(),
                        });
                    }
                }
            }
            Ok(matches)
        }
        async fn create_directory(&self, sid: SessionId, path: &str) -> Result<FileInfo> {
            Ok(FileInfo {
                id: uuid::Uuid::nil(),
                session_id: sid.uuid(),
                name: path.rsplit('/').next().unwrap_or("").to_string(),
                path: path.to_string(),
                is_directory: true,
                is_readonly: false,
                size_bytes: 0,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
        }
    }

    #[test]
    fn normalize_resolves_relative_against_cwd() {
        assert_eq!(
            normalize_virtual("foo/bar", "/workspace"),
            "/workspace/foo/bar"
        );
        assert_eq!(normalize_virtual("/foo", "/workspace"), "/foo");
        assert_eq!(normalize_virtual("a/../b", "/workspace"), "/workspace/b");
        assert_eq!(normalize_virtual("../../x", "/workspace"), "/x");
        assert_eq!(normalize_virtual(".", "/workspace"), "/workspace");
        assert_eq!(normalize_virtual("/", "/workspace"), "/");
    }

    #[tokio::test]
    async fn workspace_and_root_address_the_same_file() {
        let backend: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let fs = MountFs::new(backend);

        // Write via the /workspace view; read back via the backend-native path.
        fs.write_file(sid(), "/workspace/src/lib.rs", "X", "text")
            .await
            .unwrap();
        let via_root = fs.read_file(sid(), "/src/lib.rs").await.unwrap().unwrap();
        assert_eq!(via_root.content.as_deref(), Some("X"));
        // The backend keyed it at /src/lib.rs (no /workspace in the keyspace).
        assert_eq!(via_root.path, "/src/lib.rs");
    }

    #[tokio::test]
    async fn relative_paths_resolve_against_cwd() {
        let backend: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let fs = MountFs::new(backend);
        assert_eq!(fs.cwd(), "/workspace");

        fs.write_file(sid(), "notes.md", "hi", "text")
            .await
            .unwrap();
        // cwd is /workspace, so the relative write landed at backend /notes.md.
        let read = fs.read_file(sid(), "/notes.md").await.unwrap().unwrap();
        assert_eq!(read.content.as_deref(), Some("hi"));
    }

    #[tokio::test]
    async fn legacy_subtree_paths_pass_through_root_mount() {
        let backend: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let fs = MountFs::new(backend);
        // Internal callers write /outputs/... and /AGENTS.md directly.
        fs.write_file(sid(), "/outputs/call.stdout", "out", "text")
            .await
            .unwrap();
        let read = fs
            .read_file(sid(), "/workspace/outputs/call.stdout")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read.content.as_deref(), Some("out"));
    }

    #[test]
    fn display_uses_workspace_alias_for_primary() {
        let backend: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let fs = MountFs::new(backend);
        assert_eq!(fs.display_root(), "/workspace");
        assert_eq!(fs.display_path("/src/lib.rs"), "/workspace/src/lib.rs");
        assert_eq!(fs.display_path("/"), "/workspace");
        assert_eq!(fs.resolve_path("src/lib.rs"), "/workspace/src/lib.rs");
    }

    #[test]
    fn workspace_alias_is_the_default_even_over_a_host_backend() {
        // The server-safe default: a real-disk-style backend's host identity is
        // hidden behind /workspace unless the embedder opts in (#2776, TM-FS).
        let backend: Arc<dyn SessionFileSystem> = Arc::new(FlatStore {
            host_root: Some("/host/root".to_string()),
            ..Default::default()
        });
        let fs = MountFs::new(backend);
        assert_eq!(fs.display_root(), "/workspace");
        assert_eq!(fs.display_path("/src/lib.rs"), "/workspace/src/lib.rs");
        assert_eq!(fs.resolve_path("src/lib.rs"), "/workspace/src/lib.rs");
    }

    #[test]
    fn backend_display_exposes_host_paths_while_routing_is_unchanged() {
        // The local-embedder opt-in (yolop / #258): presentation delegates to the
        // backend's real host path, but routing still treats /workspace as cwd and
        // resolves to the same backend key.
        let backend: Arc<dyn SessionFileSystem> = Arc::new(FlatStore {
            host_root: Some("/host/root".to_string()),
            ..Default::default()
        });
        let fs = MountFs::new(backend).with_backend_display();

        assert_eq!(fs.display_root(), "/host/root");
        assert_eq!(fs.display_path("/src/lib.rs"), "/host/root/src/lib.rs");
        assert_eq!(fs.display_path("/"), "/host/root");
        // Both the relative and the /workspace-addressed spellings present the
        // same host path — routing is independent of presentation.
        assert_eq!(fs.resolve_path("src/lib.rs"), "/host/root/src/lib.rs");
        assert_eq!(
            fs.resolve_path("/workspace/src/lib.rs"),
            "/host/root/src/lib.rs"
        );
    }

    #[test]
    fn scoped_prompt_file_store_preserves_backend_native_policy() {
        // The regression guard for #258: when the embedder hands in a MountFs
        // that already opted into backend-native display, the shared prompt-store
        // wrapper must NOT bury it under a fresh `/workspace`-alias resolver.
        let backend: Arc<dyn SessionFileSystem> = Arc::new(FlatStore {
            host_root: Some("/host/root".to_string()),
            ..Default::default()
        });
        let embedder_store: Arc<dyn SessionFileSystem> =
            Arc::new(MountFs::new(backend).with_backend_display());

        let prompt_store =
            scoped_prompt_file_store(embedder_store, crate::typed_id::WorkspaceId::from_seed(1));

        // Host path survives all the way to what the system prompt would render.
        assert_eq!(prompt_store.display_root(), "/host/root");
        assert_eq!(
            prompt_store.display_path("/src/lib.rs"),
            "/host/root/src/lib.rs"
        );
        // Routing is unchanged: /workspace still resolves to the backend.
        assert_eq!(
            prompt_store.resolve_path("/workspace/src/lib.rs"),
            "/host/root/src/lib.rs"
        );
    }

    #[test]
    fn scoped_prompt_file_store_defaults_plain_backend_to_workspace_alias() {
        // A multi-tenant/server store is not a mount resolver, so the wrapper
        // still mounts it under the host-agnostic `/workspace` alias — host paths
        // must never leak into model-visible output (#2776, TM-FS).
        let backend: Arc<dyn SessionFileSystem> = Arc::new(FlatStore {
            host_root: Some("/host/root".to_string()),
            ..Default::default()
        });

        let prompt_store =
            scoped_prompt_file_store(backend, crate::typed_id::WorkspaceId::from_seed(2));

        assert_eq!(prompt_store.display_root(), WORKSPACE_MOUNT);
        assert!(
            !prompt_store
                .display_path("/src/lib.rs")
                .contains("/host/root")
        );
    }

    #[tokio::test]
    async fn display_preserves_literal_backend_workspace_segment() {
        let backend: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        backend
            .write_file(sid(), "/workspace/collide.txt", "literal", "text")
            .await
            .unwrap();
        backend
            .write_file(sid(), "/collide.txt", "alias", "text")
            .await
            .unwrap();
        let fs = MountFs::new(backend);

        let literal = fs
            .read_file(sid(), "workspace/collide.txt")
            .await
            .unwrap()
            .unwrap();
        let display_path = fs.display_path(&literal.path);
        assert_eq!(display_path, "/workspace/workspace/collide.txt");

        let round_trip = fs.read_file(sid(), &display_path).await.unwrap().unwrap();
        assert_eq!(round_trip.content.as_deref(), Some("literal"));
    }

    #[tokio::test]
    async fn additional_mount_routes_to_its_backend() {
        let workspace: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let volume: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let fs = MountFs::new(workspace).with_mount("/data", volume.clone(), "/");

        fs.write_file(sid(), "/data/report.csv", "1,2,3", "text")
            .await
            .unwrap();
        // It went to the volume backend at /report.csv, not the workspace.
        let from_volume = volume
            .read_file(sid(), "/report.csv")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(from_volume.content.as_deref(), Some("1,2,3"));
    }

    #[tokio::test]
    async fn additional_mount_outputs_use_virtual_paths() {
        let workspace: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let volume: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let fs = MountFs::new(workspace).with_mount("/workspace/roots/backend", volume, "/");

        let written = fs
            .write_file(
                sid(),
                "/workspace/roots/backend/Cargo.toml",
                "name = \"backend\"",
                "text",
            )
            .await
            .unwrap();
        assert_eq!(written.path, "/workspace/roots/backend/Cargo.toml");

        let stat = fs
            .stat_file(sid(), "/workspace/roots/backend/Cargo.toml")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stat.path, "/workspace/roots/backend/Cargo.toml");
        assert_eq!(
            fs.display_path(&stat.path),
            "/workspace/roots/backend/Cargo.toml"
        );
        assert_eq!(
            fs.resolve_path("/workspace/roots/backend/Cargo.toml"),
            "/workspace/roots/backend/Cargo.toml"
        );
    }

    #[tokio::test]
    async fn grep_without_path_searches_all_mounts() {
        let workspace: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let volume: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let fs = MountFs::new(workspace).with_mount("/workspace/roots/backend", volume, "/");

        fs.write_file(sid(), "/workspace/README.md", "needle primary", "text")
            .await
            .unwrap();
        fs.write_file(
            sid(),
            "/workspace/roots/backend/Cargo.toml",
            "needle backend",
            "text",
        )
        .await
        .unwrap();

        let matches = fs.grep_files(sid(), "needle", None).await.unwrap();
        let paths: Vec<_> = matches.into_iter().map(|m| m.path).collect();
        assert_eq!(
            paths,
            vec![
                "/README.md".to_string(),
                "/workspace/roots/backend/Cargo.toml".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn grep_resolves_workspace_glob_to_backend_namespace() {
        let backend: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let fs = MountFs::new(backend);
        fs.write_file(sid(), "/workspace/src/lib.rs", "needle", "text")
            .await
            .unwrap();
        fs.write_file(sid(), "/workspace/docs/readme.md", "needle", "text")
            .await
            .unwrap();

        let matches = fs
            .grep_files(sid(), "needle", Some("/workspace/src/**/*.rs"))
            .await
            .unwrap();

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path, "/src/lib.rs");
    }

    #[tokio::test]
    async fn grep_glob_searches_every_matching_mount() {
        let workspace: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let volume: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let fs = MountFs::new(workspace).with_mount("/workspace/roots/backend", volume, "/");
        fs.write_file(sid(), "/workspace/Cargo.toml", "needle", "text")
            .await
            .unwrap();
        fs.write_file(
            sid(),
            "/workspace/roots/backend/Cargo.toml",
            "needle",
            "text",
        )
        .await
        .unwrap();

        let paths: Vec<_> = fs
            .grep_files(sid(), "needle", Some("**/*.toml"))
            .await
            .unwrap()
            .into_iter()
            .map(|hit| hit.path)
            .collect();
        assert_eq!(
            paths,
            vec![
                "/Cargo.toml".to_string(),
                "/workspace/roots/backend/Cargo.toml".to_string()
            ]
        );
    }

    #[test]
    fn mount_fs_identifies_as_resolver() {
        let workspace: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let fs = MountFs::wrap(workspace);
        assert!(fs.is_mount_resolver());
        let again = MountFs::wrap_if_needed(fs.clone());
        assert!(Arc::ptr_eq(&fs, &again));
    }
}
