// Virtual mount registry — per-session registry of virtual file trees.
//
// Virtual mounts are registered during apply_capability_mounts() for
// MountSource::Virtual entries. Content is served from memory without
// writing rows to session_files. Evicted on session delete.

use everruns_core::capability_types::VirtualFileTree;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// A registered virtual mount for a session.
struct RegisteredVirtualMount {
    mount_path: String,
    tree: Arc<VirtualFileTree>,
    #[allow(dead_code)]
    capability_id: String,
}

/// Session-scoped registry of virtual mount trees.
pub struct VirtualMountRegistry {
    mounts: RwLock<HashMap<Uuid, Vec<RegisteredVirtualMount>>>,
}

impl VirtualMountRegistry {
    pub fn new() -> Self {
        Self {
            mounts: RwLock::new(HashMap::new()),
        }
    }

    /// Register a virtual mount for a session.
    pub fn register(
        &self,
        session_id: Uuid,
        mount_path: String,
        tree: Arc<VirtualFileTree>,
        capability_id: String,
    ) {
        self.mounts
            .write()
            .entry(session_id)
            .or_default()
            .push(RegisteredVirtualMount {
                mount_path,
                tree,
                capability_id,
            });
    }

    /// Remove all virtual mounts for a session.
    pub fn evict(&self, session_id: &Uuid) {
        self.mounts.write().remove(session_id);
    }

    /// Read a file from virtual mounts. Returns content if the path matches any mount.
    pub fn read_file(&self, session_id: &Uuid, path: &str) -> Option<VirtualFileRead> {
        let mounts = self.mounts.read();
        let session_mounts = mounts.get(session_id)?;
        for mount in session_mounts {
            if let Some(relative) = strip_mount_prefix(path, &mount.mount_path) {
                let lookup = if relative.is_empty() {
                    mount.mount_path.clone()
                } else {
                    format!("{}/{relative}", mount.mount_path)
                };
                if let Some(file) = mount.tree.get(&lookup) {
                    return Some(VirtualFileRead {
                        content: file.content.clone(),
                        is_directory: file.is_directory,
                        path: path.to_string(),
                    });
                }
            }
        }
        None
    }

    /// List virtual entries under a directory path.
    pub fn list_directory(&self, session_id: &Uuid, dir_path: &str) -> Vec<VirtualFileRead> {
        let mut results = Vec::new();
        let mounts = self.mounts.read();
        let Some(session_mounts) = mounts.get(session_id) else {
            return results;
        };
        for mount in session_mounts {
            if strip_mount_prefix(dir_path, &mount.mount_path).is_some() {
                for (entry_path, file) in mount.tree.list_directory(dir_path) {
                    results.push(VirtualFileRead {
                        content: file.content.clone(),
                        is_directory: file.is_directory,
                        path: entry_path,
                    });
                }
            } else if strip_mount_prefix(&mount.mount_path, dir_path).is_some() {
                // The mount root itself is inside the listed directory
                if let Some(file) = mount.tree.get(&mount.mount_path) {
                    results.push(VirtualFileRead {
                        content: file.content.clone(),
                        is_directory: file.is_directory,
                        path: mount.mount_path.clone(),
                    });
                }
            }
        }
        results
    }

    /// Check if a path falls under any virtual mount (for write protection).
    pub fn is_virtual_path(&self, session_id: &Uuid, path: &str) -> bool {
        let mounts = self.mounts.read();
        let Some(session_mounts) = mounts.get(session_id) else {
            return false;
        };
        session_mounts
            .iter()
            .any(|m| strip_mount_prefix(path, &m.mount_path).is_some())
    }

    /// Search virtual files for grep matches.
    pub fn grep(
        &self,
        session_id: &Uuid,
        pattern: &regex::Regex,
        path_filter: Option<&str>,
    ) -> Vec<VirtualGrepMatch> {
        let mut results = Vec::new();
        let mounts = self.mounts.read();
        let Some(session_mounts) = mounts.get(session_id) else {
            return results;
        };
        for mount in session_mounts {
            for (file_path, file) in mount.tree.all_files() {
                if let Some(filter) = path_filter {
                    let filter_prefix = if filter.ends_with('/') {
                        filter.to_string()
                    } else {
                        format!("{filter}/")
                    };
                    if !file_path.starts_with(&filter_prefix) && file_path != filter {
                        continue;
                    }
                }
                let content = match std::str::from_utf8(&file.content) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                for (line_num, line) in content.lines().enumerate() {
                    if pattern.is_match(line) {
                        results.push(VirtualGrepMatch {
                            path: file_path.to_string(),
                            line_number: line_num + 1,
                            line: line.to_string(),
                        });
                    }
                }
            }
        }
        results
    }
}

impl Default for VirtualMountRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of reading a virtual file.
pub struct VirtualFileRead {
    pub content: Vec<u8>,
    pub is_directory: bool,
    pub path: String,
}

/// A grep match from virtual files.
pub struct VirtualGrepMatch {
    pub path: String,
    pub line_number: usize,
    pub line: String,
}

/// Strip the mount prefix from a path. Returns the relative portion.
/// E.g. strip_mount_prefix("/docs/foo.md", "/docs") -> Some("foo.md")
/// E.g. strip_mount_prefix("/docs", "/docs") -> Some("")
fn strip_mount_prefix<'a>(path: &'a str, mount_path: &str) -> Option<&'a str> {
    if path == mount_path {
        Some("")
    } else if let Some(rest) = path.strip_prefix(mount_path) {
        rest.strip_prefix('/').or(Some(rest))
    } else {
        None
    }
}
