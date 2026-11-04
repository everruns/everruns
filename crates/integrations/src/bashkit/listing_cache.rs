//! What a [`SessionFileSystemAdapter`] already listed.
//!
//! Decision: every store call is a round trip to the control plane (about
//! 10 ms from a worker). `find`, `ls -R` and `rg` list every directory and
//! stat every entry, so a walk over a 3.7k-file checkout took over 30 s.
//! Listings carry each child's type and size, so `stat` answers from them,
//! and once a walk goes below a directory it listed, the adapter fetches
//! that whole subtree in one `list_tree` call and serves the rest of the
//! walk from it. The adapter lives for one `bash` call; any change through
//! it drops everything remembered, and writes by other tools during the
//! same call are not seen until the next call.

use super::SessionFileSystemAdapter;
use crate::bashkit::session_file::FileInfo;
use bashkit::{DirEntry, FileType, Metadata};
use everruns_contracts::runtime::session_files::SessionFileSystem;
use std::path::Path;
use std::time::SystemTime;

pub(super) struct ForgetListings<'a>(&'a std::sync::Mutex<ListingCache>);

impl ForgetListings<'_> {
    fn forget(&self) {
        if let Ok(mut listings) = self.0.lock() {
            *listings = ListingCache::default();
        }
    }
}

impl Drop for ForgetListings<'_> {
    fn drop(&mut self) {
        self.forget();
    }
}

#[derive(Default)]
pub(super) struct ListingCache {
    metadata: std::collections::HashMap<String, Metadata>,
    listed_dirs: std::collections::HashSet<String>,
    children: std::collections::HashMap<String, Vec<DirEntry>>,
    tree_roots_tried: std::collections::HashSet<String>,
}

/// Entries remembered per `bash` call; past this the adapter asks the store.
const LISTED_METADATA_LIMIT: usize = 200_000;

fn parent_dir(path: &str) -> Option<&str> {
    let path = path.trim_end_matches('/');
    let (parent, _) = path.rsplit_once('/')?;
    Some(if parent.is_empty() { "/" } else { parent })
}

fn child_path(dir: &str, name: &str) -> String {
    format!("{}/{name}", dir.trim_end_matches('/'))
}

impl ListingCache {
    fn remember(&mut self, dir: &str, entries: &[DirEntry]) {
        if self.metadata.len() + entries.len() > LISTED_METADATA_LIMIT {
            return;
        }
        self.listed_dirs.insert(dir.to_string());
        for entry in entries {
            self.metadata
                .insert(child_path(dir, &entry.name), entry.metadata.clone());
        }
        self.children.insert(dir.to_string(), entries.to_vec());
    }

    /// The highest directory listed on the way down to `dir`, when `dir`'s
    /// parent was listed: that is a recursive walk worth one tree fetch.
    fn walk_root(&self, dir: &str) -> Option<String> {
        let mut root = parent_dir(dir).filter(|p| self.listed_dirs.contains(*p))?;
        while let Some(parent) = parent_dir(root).filter(|p| self.listed_dirs.contains(*p)) {
            if parent == root {
                break;
            }
            root = parent;
        }
        (!self.tree_roots_tried.contains(root)).then(|| root.to_string())
    }

    /// Index a subtree listing (paths relative to `root`) as per-directory
    /// children.
    fn remember_tree(&mut self, root: &str, infos: Vec<FileInfo>, now: SystemTime) {
        if infos.len() > LISTED_METADATA_LIMIT {
            return;
        }
        let root = root.trim_end_matches('/');
        let mut children: std::collections::HashMap<
            String,
            std::collections::BTreeMap<String, Metadata>,
        > = std::collections::HashMap::new();
        let directory = Metadata {
            file_type: FileType::Directory,
            size: 0,
            mode: 0o755,
            modified: now,
            created: now,
        };
        for info in infos {
            let rest = info.path.trim_start_matches('/');
            let metadata = Metadata {
                file_type: if info.is_directory {
                    FileType::Directory
                } else {
                    FileType::File
                },
                size: info.size_bytes as u64,
                mode: 0o755,
                modified: now,
                created: now,
            };
            // Every ancestor is a directory, whether or not the store has a row for it.
            let mut dir = root.to_string();
            let mut parts = rest.split('/').filter(|part| !part.is_empty()).peekable();
            while let Some(part) = parts.next() {
                let meta = if parts.peek().is_some() {
                    directory.clone()
                } else {
                    metadata.clone()
                };
                let entry = children.entry(dir.clone()).or_default();
                let slot = entry
                    .entry(part.to_string())
                    .or_insert_with(|| meta.clone());
                if meta.file_type == FileType::Directory {
                    *slot = meta;
                    children.entry(child_path(&dir, part)).or_default();
                }
                dir = child_path(&dir, part);
            }
        }
        children.entry(root.to_string()).or_default();
        for (dir, entries) in children {
            let entries: Vec<DirEntry> = entries
                .into_iter()
                .map(|(name, metadata)| DirEntry { name, metadata })
                .collect();
            self.remember(&dir, &entries);
        }
    }
}

impl SessionFileSystemAdapter {
    pub(super) fn listed_metadata(&self, path: &Path) -> Option<Metadata> {
        self.listings
            .lock()
            .ok()?
            .metadata
            .get(path.to_string_lossy().as_ref())
            .cloned()
    }

    /// Any change through this adapter can make a remembered entry wrong.
    /// Forgets now, and again when the returned guard drops at the end of the
    /// change, since the change itself may list or stat on the way.
    pub(super) fn forget_listings(&self) -> ForgetListings<'_> {
        ForgetListings(&self.listings).forget();
        ForgetListings(&self.listings)
    }

    /// Serve `dir` from a fetched subtree, fetching it first when this is a
    /// recursive walk below a directory already listed.
    async fn listing_from_tree(&self, dir: &str) -> Option<Vec<DirEntry>> {
        let root = {
            let mut listings = self.listings.lock().ok()?;
            if let Some(entries) = listings.children.get(dir) {
                return Some(entries.clone());
            }
            let root = listings.walk_root(dir)?;
            listings.tree_roots_tried.insert(root.clone());
            root
        };
        let infos = self
            .store
            .list_tree(self.session_id, &root)
            .await
            .ok()
            .flatten()?;
        let mut listings = self.listings.lock().ok()?;
        listings.remember_tree(&root, infos, SystemTime::now());
        listings.children.get(dir).cloned()
    }

    /// `read_dir` without the bashkit error mapping, served from what this
    /// adapter already listed when it can.
    pub(super) async fn list_dir(
        &self,
        session_path: &str,
    ) -> everruns_contracts::error::Result<Vec<DirEntry>> {
        if let Some(entries) = self.listing_from_tree(session_path).await {
            return Ok(entries);
        }
        let entries = self
            .store
            .list_directory(self.session_id, session_path)
            .await?;
        let now = SystemTime::now();
        let entries: Vec<DirEntry> = entries
            .into_iter()
            .map(|e| DirEntry {
                name: e.name,
                metadata: Metadata {
                    file_type: if e.is_directory {
                        FileType::Directory
                    } else {
                        FileType::File
                    },
                    size: e.size_bytes as u64,
                    mode: 0o755,
                    modified: now,
                    created: now,
                },
            })
            .collect();
        if let Ok(mut listings) = self.listings.lock() {
            listings.remember(session_path, &entries);
        }
        Ok(entries)
    }

    /// Whether `session_path` is an implicit directory — one with children but no row
    /// of its own (virtual mounts, unmaterialized parents).
    ///
    /// Stores answer `list_directory` for an unknown path with `Ok(vec![])`, so only a
    /// non-empty listing distinguishes a real directory from an absent path.
    pub(super) async fn directory_has_entries(&self, session_path: &str) -> bool {
        self.list_dir(session_path)
            .await
            .is_ok_and(|entries| !entries.is_empty())
    }
}
