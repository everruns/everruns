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

    /// List virtual entries under a directory path (metadata only, no content cloned).
    pub fn list_directory(&self, session_id: &Uuid, dir_path: &str) -> Vec<VirtualFileMeta> {
        let mut results = Vec::new();
        let mounts = self.mounts.read();
        let Some(session_mounts) = mounts.get(session_id) else {
            return results;
        };
        for mount in session_mounts {
            if strip_mount_prefix(dir_path, &mount.mount_path).is_some() {
                for (entry_path, file) in mount.tree.list_directory(dir_path) {
                    results.push(VirtualFileMeta {
                        size_bytes: file.content.len() as i64,
                        is_directory: file.is_directory,
                        path: entry_path,
                    });
                }
            } else if strip_mount_prefix(&mount.mount_path, dir_path).is_some() {
                // The mount root itself is inside the listed directory
                if let Some(file) = mount.tree.get(&mount.mount_path) {
                    results.push(VirtualFileMeta {
                        size_bytes: file.content.len() as i64,
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

/// Metadata-only result for directory listings (avoids cloning file content).
pub struct VirtualFileMeta {
    pub path: String,
    pub is_directory: bool,
    pub size_bytes: i64,
}

/// A grep match from virtual files.
pub struct VirtualGrepMatch {
    pub path: String,
    pub line_number: usize,
    pub line: String,
}

/// Strip the mount prefix from a path. Returns the relative portion.
/// Only matches on segment boundaries (not arbitrary string prefixes).
/// E.g. strip_mount_prefix("/docs/foo.md", "/docs") -> Some("foo.md")
/// E.g. strip_mount_prefix("/docs", "/docs") -> Some("")
/// E.g. strip_mount_prefix("/docs2", "/docs") -> None
fn strip_mount_prefix<'a>(path: &'a str, mount_path: &str) -> Option<&'a str> {
    if path == mount_path {
        Some("")
    } else if mount_path == "/" {
        path.strip_prefix('/')
    } else {
        let prefix = format!("{mount_path}/");
        path.strip_prefix(&prefix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::capability_types::VirtualFileTree;

    #[test]
    fn strip_mount_prefix_exact_match() {
        assert_eq!(strip_mount_prefix("/docs", "/docs"), Some(""));
    }

    #[test]
    fn strip_mount_prefix_child_path() {
        assert_eq!(strip_mount_prefix("/docs/foo.md", "/docs"), Some("foo.md"));
    }

    #[test]
    fn strip_mount_prefix_no_match_partial_segment() {
        // "/docs2" should NOT match mount "/docs"
        assert_eq!(strip_mount_prefix("/docs2", "/docs"), None);
        assert_eq!(strip_mount_prefix("/docsfoo", "/docs"), None);
    }

    #[test]
    fn strip_mount_prefix_root_mount() {
        assert_eq!(strip_mount_prefix("/foo.md", "/"), Some("foo.md"));
        assert_eq!(strip_mount_prefix("/", "/"), Some(""));
    }

    #[test]
    fn strip_mount_prefix_no_match_different_prefix() {
        assert_eq!(strip_mount_prefix("/other/foo", "/docs"), None);
    }

    #[test]
    fn registry_read_file() {
        let registry = VirtualMountRegistry::new();
        let sid = Uuid::new_v4();
        let mut tree = VirtualFileTree::new();
        tree.insert_text("/mnt/hello.txt", "world");
        registry.register(sid, "/mnt".into(), Arc::new(tree), "cap".into());

        let result = registry.read_file(&sid, "/mnt/hello.txt").unwrap();
        assert_eq!(result.content, b"world");
        assert!(!result.is_directory);
    }

    #[test]
    fn registry_read_file_no_partial_segment_match() {
        let registry = VirtualMountRegistry::new();
        let sid = Uuid::new_v4();
        let mut tree = VirtualFileTree::new();
        tree.insert_text("/mnt/hello.txt", "world");
        registry.register(sid, "/mnt".into(), Arc::new(tree), "cap".into());

        // "/mnt2/hello.txt" should NOT match mount at "/mnt"
        assert!(registry.read_file(&sid, "/mnt2/hello.txt").is_none());
    }

    #[test]
    fn registry_is_virtual_path_no_partial_segment_match() {
        let registry = VirtualMountRegistry::new();
        let sid = Uuid::new_v4();
        let mut tree = VirtualFileTree::new();
        tree.insert_text("/docs/a.md", "content");
        registry.register(sid, "/docs".into(), Arc::new(tree), "cap".into());

        assert!(registry.is_virtual_path(&sid, "/docs/a.md"));
        assert!(registry.is_virtual_path(&sid, "/docs"));
        assert!(!registry.is_virtual_path(&sid, "/docs2"));
        assert!(!registry.is_virtual_path(&sid, "/docsfoo"));
    }

    #[test]
    fn registry_list_directory_returns_metadata() {
        let registry = VirtualMountRegistry::new();
        let sid = Uuid::new_v4();
        let mut tree = VirtualFileTree::new();
        tree.insert_text("/docs/a.md", "aaa");
        tree.insert_text("/docs/b.md", "bbb");
        registry.register(sid, "/docs".into(), Arc::new(tree), "cap".into());

        let entries = registry.list_directory(&sid, "/docs");
        assert_eq!(entries.len(), 2);
        // Verify we get metadata, not content
        for entry in &entries {
            assert!(entry.size_bytes > 0);
        }
    }

    #[test]
    fn registry_evict_removes_session() {
        let registry = VirtualMountRegistry::new();
        let sid = Uuid::new_v4();
        let mut tree = VirtualFileTree::new();
        tree.insert_text("/mnt/hello.txt", "world");
        registry.register(sid, "/mnt".into(), Arc::new(tree), "cap".into());

        assert!(registry.read_file(&sid, "/mnt/hello.txt").is_some());
        registry.evict(&sid);
        assert!(registry.read_file(&sid, "/mnt/hello.txt").is_none());
    }

    #[test]
    fn registry_grep_matches() {
        let registry = VirtualMountRegistry::new();
        let sid = Uuid::new_v4();
        let mut tree = VirtualFileTree::new();
        tree.insert_text("/docs/readme.md", "line 1\nfind me\nline 3");
        registry.register(sid, "/docs".into(), Arc::new(tree), "cap".into());

        let pattern = regex::Regex::new("find me").unwrap();
        let results = registry.grep(&sid, &pattern, None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].line_number, 2);
        assert_eq!(results[0].line, "find me");
    }
}
