use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use crate::error::{AgentLoopError, Result};
use crate::mount_fs::WORKSPACE_MOUNT;

pub const PRIMARY_WORKSPACE_ROOT_NAME: &str = "workspace";
pub const ADDITIONAL_ROOTS_MOUNT: &str = "/workspace/roots";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceRoot {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceRootSet {
    pub primary: WorkspaceRoot,
    pub additional: Vec<WorkspaceRoot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPath {
    pub root_name: String,
    pub relative: RelPath,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct RelPath(String);

impl RelPath {
    pub fn as_relative(&self) -> &str {
        &self.0
    }

    pub fn to_session_path(&self) -> String {
        if self.0.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", self.0)
        }
    }

    fn from_path(relative: &Path) -> Result<Self> {
        let mut segments = Vec::new();
        for component in relative.components() {
            match component {
                Component::CurDir => {}
                Component::Normal(segment) => {
                    let segment = segment.to_str().ok_or_else(|| {
                        AgentLoopError::tool(format!(
                            "non-UTF-8 path component: {}",
                            relative.display()
                        ))
                    })?;
                    segments.push(segment.to_string());
                }
                Component::ParentDir => {
                    return Err(AgentLoopError::tool(format!(
                        "path traversal rejected: {}",
                        relative.display()
                    )));
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(AgentLoopError::tool(format!(
                        "absolute path component rejected: {}",
                        relative.display()
                    )));
                }
            }
        }
        Ok(Self(segments.join("/")))
    }

    fn from_str(input: &str) -> Result<Self> {
        let mut segments = Vec::new();
        for part in input.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    return Err(AgentLoopError::tool(format!(
                        "path traversal rejected: {input}"
                    )));
                }
                segment => segments.push(segment.to_string()),
            }
        }
        Ok(Self(segments.join("/")))
    }
}

impl WorkspaceRootSet {
    pub fn from_primary(path: impl Into<PathBuf>) -> Result<Self> {
        Self::new(path, Vec::<(String, PathBuf)>::new())
    }

    pub fn new<I, P>(primary: impl Into<PathBuf>, additional: I) -> Result<Self>
    where
        I: IntoIterator<Item = (String, P)>,
        P: Into<PathBuf>,
    {
        let primary = WorkspaceRoot {
            name: PRIMARY_WORKSPACE_ROOT_NAME.to_string(),
            path: canonicalize_root(primary.into())?,
        };
        let mut additional_roots = Vec::new();
        let mut names = HashSet::new();
        for (name, path) in additional {
            validate_additional_name(&name)?;
            if !names.insert(name.clone()) {
                return Err(AgentLoopError::config(format!(
                    "duplicate workspace root name: {name}"
                )));
            }
            additional_roots.push(WorkspaceRoot {
                name,
                path: canonicalize_root(path.into())?,
            });
        }

        let root_set = Self {
            primary,
            additional: additional_roots,
        };
        root_set.reject_overlaps()?;
        Ok(root_set)
    }

    pub fn parse_vfs_path(&self, input: &str) -> Result<ResolvedPath> {
        let trimmed = input.trim();
        let candidate = Path::new(trimmed);
        if candidate.is_absolute() && !trimmed.starts_with("/workspace") {
            if let Some((root, relative)) = self.resolve_host_path(candidate)? {
                return Ok(ResolvedPath {
                    root_name: root.name.clone(),
                    relative,
                });
            }
            return Err(AgentLoopError::tool(format!(
                "host path is outside registered workspace roots: {trimmed}"
            )));
        }

        let session = if trimmed == WORKSPACE_MOUNT || trimmed == "workspace" {
            "/".to_string()
        } else if let Some(rest) = trimmed.strip_prefix("/workspace/") {
            format!("/{rest}")
        } else if trimmed.starts_with('/') {
            trimmed.to_string()
        } else {
            format!("/{trimmed}")
        };

        if session == "/" || !session.starts_with("/roots/") {
            return Ok(ResolvedPath {
                root_name: self.primary.name.clone(),
                relative: RelPath::from_str(&session)?,
            });
        }

        let rest = session.strip_prefix("/roots/").unwrap_or_default();
        let (name, relative) = rest.split_once('/').unwrap_or((rest, ""));
        let root = self
            .additional
            .iter()
            .find(|root| root.name == name)
            .ok_or_else(|| AgentLoopError::tool(format!("unknown workspace root: {name}")))?;
        Ok(ResolvedPath {
            root_name: root.name.clone(),
            relative: RelPath::from_str(relative)?,
        })
    }

    pub fn parse_host_scope(&self, root: &str, relative: Option<&str>) -> Result<PathBuf> {
        let workspace_root = self.root_by_name(root)?;
        let rel = RelPath::from_str(relative.unwrap_or(""))?;
        let candidate = if rel.as_relative().is_empty() {
            workspace_root.path.clone()
        } else {
            workspace_root.path.join(rel.as_relative())
        };
        let resolved = canonicalize_existing_or_parent(&candidate)?;
        if resolved != workspace_root.path && !resolved.starts_with(&workspace_root.path) {
            return Err(AgentLoopError::tool(format!(
                "path escapes workspace root: {}",
                candidate.display()
            )));
        }
        Ok(resolved)
    }

    pub fn primary_host_root(&self) -> &Path {
        &self.primary.path
    }

    pub fn set_primary_host_root(&mut self, path: PathBuf) -> Result<()> {
        // A rejected reconfiguration must leave the registered roots usable.
        let mut updated = self.clone();
        updated.primary.path = canonicalize_root(path)?;
        updated.reject_overlaps()?;
        *self = updated;
        Ok(())
    }

    pub fn spawn_cwd(&self) -> Result<PathBuf> {
        canonicalize_root(self.primary.path.clone())
    }

    pub fn contains_host_path(&self, path: &Path) -> bool {
        let Ok(canonical) = canonicalize_existing_or_parent(path) else {
            return false;
        };
        self.all_roots()
            .any(|root| canonical == root.path || canonical.starts_with(&root.path))
    }

    pub fn additional_mount_point(root_name: &str) -> String {
        format!("{ADDITIONAL_ROOTS_MOUNT}/{root_name}")
    }

    fn all_roots(&self) -> impl Iterator<Item = &WorkspaceRoot> {
        std::iter::once(&self.primary).chain(self.additional.iter())
    }

    fn root_by_name(&self, name: &str) -> Result<&WorkspaceRoot> {
        if name == self.primary.name {
            return Ok(&self.primary);
        }
        self.additional
            .iter()
            .find(|root| root.name == name)
            .ok_or_else(|| AgentLoopError::tool(format!("unknown workspace root: {name}")))
    }

    fn resolve_host_path(&self, path: &Path) -> Result<Option<(&WorkspaceRoot, RelPath)>> {
        let canonical = canonicalize_existing_or_parent(path)?;
        for root in self.all_roots() {
            if let Ok(relative) = canonical.strip_prefix(&root.path) {
                return Ok(Some((root, RelPath::from_path(relative)?)));
            }
        }
        Ok(None)
    }

    fn reject_overlaps(&self) -> Result<()> {
        let roots: Vec<&WorkspaceRoot> = self.all_roots().collect();
        for (idx, left) in roots.iter().enumerate() {
            for right in roots.iter().skip(idx + 1) {
                if left.path == right.path
                    || left.path.starts_with(&right.path)
                    || right.path.starts_with(&left.path)
                {
                    return Err(AgentLoopError::config(format!(
                        "workspace roots must not overlap: {} ({}) and {} ({})",
                        left.name,
                        left.path.display(),
                        right.name,
                        right.path.display()
                    )));
                }
            }
        }
        Ok(())
    }
}

fn validate_additional_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name == PRIMARY_WORKSPACE_ROOT_NAME
        || name == "roots"
        || name.contains('/')
        || name.contains('\\')
    {
        return Err(AgentLoopError::config(format!(
            "invalid workspace root name: {name}"
        )));
    }
    Ok(())
}

fn canonicalize_root(root: PathBuf) -> Result<PathBuf> {
    let canonical = std::fs::canonicalize(&root).map_err(|e| {
        AgentLoopError::config(format!(
            "failed to canonicalize workspace root {}: {e}",
            root.display()
        ))
    })?;
    if !canonical.is_dir() {
        return Err(AgentLoopError::config(format!(
            "workspace root is not a directory: {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn canonicalize_existing_or_parent(path: &Path) -> Result<PathBuf> {
    match std::fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(_) => {
            let parent = path.parent().ok_or_else(|| {
                AgentLoopError::tool(format!("path has no parent: {}", path.display()))
            })?;
            let canonical_parent = std::fs::canonicalize(parent).map_err(|e| {
                AgentLoopError::tool(format!(
                    "failed to canonicalize parent {}: {e}",
                    parent.display()
                ))
            })?;
            let name = path.file_name().ok_or_else(|| {
                AgentLoopError::tool(format!("path has no file name: {}", path.display()))
            })?;
            Ok(canonical_parent.join(name))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn roots() -> (WorkspaceRootSet, TempDir, TempDir) {
        let primary = TempDir::new().unwrap();
        let backend = TempDir::new().unwrap();
        let set = WorkspaceRootSet::new(
            primary.path(),
            [("backend".to_string(), backend.path().to_path_buf())],
        )
        .unwrap();
        (set, primary, backend)
    }

    #[test]
    fn canonicalizes_and_rejects_overlapping_roots() {
        let primary = TempDir::new().unwrap();
        let nested = primary.path().join("nested");
        std::fs::create_dir(&nested).unwrap();

        for (first, second) in [
            (primary.path(), nested.as_path()),
            (nested.as_path(), primary.path()),
            (primary.path(), primary.path()),
        ] {
            let err = WorkspaceRootSet::new(first, [("other".to_string(), second)]).unwrap_err();
            assert!(err.to_string().contains("must not overlap"));
        }
        let canonical = WorkspaceRootSet::from_primary(primary.path().join("nested/..")).unwrap();
        assert_eq!(
            canonical.primary.path,
            std::fs::canonicalize(primary.path()).unwrap()
        );
    }

    #[test]
    fn rejects_duplicate_names() {
        let primary = TempDir::new().unwrap();
        let a = TempDir::new().unwrap();
        let b = TempDir::new().unwrap();

        let err = WorkspaceRootSet::new(
            primary.path(),
            [
                ("backend".to_string(), a.path().to_path_buf()),
                ("backend".to_string(), b.path().to_path_buf()),
            ],
        )
        .unwrap_err();
        assert!(err.to_string().contains("duplicate workspace root name"));
    }

    #[test]
    fn parses_primary_aliases_and_rejects_traversal() {
        let (set, _primary, _backend) = roots();

        assert_eq!(
            set.parse_vfs_path("/workspace/src/lib.rs").unwrap(),
            ResolvedPath {
                root_name: "workspace".to_string(),
                relative: RelPath("src/lib.rs".to_string())
            }
        );
        assert_eq!(
            set.parse_vfs_path("workspace").unwrap().relative,
            RelPath::default()
        );
        assert!(set.parse_vfs_path("../outside").is_err());
    }

    #[test]
    fn parses_additional_root_mounts_only() {
        let (set, _primary, _backend) = roots();

        assert_eq!(
            set.parse_vfs_path("/workspace/roots/backend/src/lib.rs")
                .unwrap(),
            ResolvedPath {
                root_name: "backend".to_string(),
                relative: RelPath("src/lib.rs".to_string())
            }
        );
        assert!(set.parse_vfs_path("/workspace/roots/missing/file").is_err());
    }

    #[test]
    fn repoints_primary_without_touching_additional() {
        let (mut set, _primary, backend) = roots();
        let next = TempDir::new().unwrap();
        set.set_primary_host_root(next.path().to_path_buf())
            .unwrap();

        assert_eq!(
            set.spawn_cwd().unwrap(),
            std::fs::canonicalize(next.path()).unwrap()
        );
        assert_eq!(
            set.parse_host_scope("backend", Some("Cargo.toml")).unwrap(),
            std::fs::canonicalize(backend.path())
                .unwrap()
                .join("Cargo.toml")
        );
    }

    #[test]
    fn rejected_primary_repoint_preserves_registered_roots() {
        let (mut set, _primary, backend) = roots();
        let original = set.clone();
        let error = set
            .set_primary_host_root(backend.path().to_path_buf())
            .unwrap_err();
        assert!(error.to_string().contains("must not overlap"));
        assert_eq!(set, original);
        assert_eq!(set.spawn_cwd().unwrap(), original.primary.path);
    }

    #[cfg(unix)]
    #[test]
    fn host_scope_rejects_symlink_escape() {
        let primary = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        std::os::unix::fs::symlink(outside.path(), primary.path().join("outside-link")).unwrap();
        let set = WorkspaceRootSet::from_primary(primary.path()).unwrap();

        let err = set
            .parse_host_scope("workspace", Some("outside-link/secret.txt"))
            .unwrap_err();

        assert!(err.to_string().contains("path escapes workspace root"));
    }
}
