//! A session's workspace in the cell (data plane v2 step 6, "a minimal
//! durable workspace"; design in `knowledge/execution/data-plane-v2/fs.md`).
//!
//! Decisions:
//! - A workspace is a path-ordered map of entries plus a version, held in
//!   the session's replicated state and changed only by [`Command::Files`]
//!   commits, so a committed write is on a quorum like an event, moves with
//!   its session, and survives any node.
//! - Contents of 16 KiB and under ride inline in the commit (one Raft
//!   entry); larger contents go to the object store by SHA-256 first
//!   (`blobs/{sha256}`, idempotent) and the commit carries the reference.
//!   Blobs are never deleted here (fs.md, "Deleting bytes").
//! - A commit names an operation id the writer chose; the cell keeps the
//!   reply of recent commits by id, so a retry after a lost acknowledgement
//!   gets the original result instead of a second change.
//! - Writers are fenced: an executor commits under the epoch it holds, and
//!   a commit under an older epoch is refused (`Fenced`), so an executor
//!   that lost its session cannot write into it. A commit without a fence
//!   is the gateway's or the control plane's (starter files, uploads).
//! - Compare-and-set is per path: a write may name the version of the entry
//!   it read, and is refused as a conflict if the entry changed since.
//! - The console's file calls (`/v1/workspaces/{id}/fs`) reach a node as
//!   one change each (`api_files.rs`); create, move and copy refuse an
//!   existing target here, in apply, so two writers cannot both win.
//! - Not here yet (fs.md steps 5 and 8): compaction of the map into tree
//!   nodes in S3, and forks. The per-session quota bounds what the map can
//!   hold until then.
//!
//! [`Command::Files`]: crate::state::Command::Files

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Contents up to this size ride inline in the commit.
pub const INLINE_MAX: usize = 16 * 1024;
/// Bytes one session's workspace may hold (inline and blobs).
pub const QUOTA: u64 = 256 * 1024 * 1024;
/// Commits whose replies are kept for retries.
const KEPT_OPS: usize = 256;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Files {
    /// Bumped by every commit that changed something.
    #[serde(default)]
    pub version: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub entries: BTreeMap<String, Entry>,
    /// Bytes held, inline and in blobs, against [`QUOTA`].
    #[serde(default)]
    pub bytes: u64,
    /// Recent commits' replies by operation id, oldest first.
    #[serde(default, skip_serializing_if = "VecDeque::is_empty")]
    pub ops: VecDeque<(String, Value)>,
}

impl Files {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.ops.is_empty() && self.version == 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    #[serde(default)]
    pub dir: bool,
    #[serde(default)]
    pub readonly: bool,
    /// `text` or `base64`, as the writer sent it.
    #[serde(default = "text")]
    pub encoding: String,
    #[serde(default)]
    pub content: Content,
    #[serde(default)]
    pub size: u64,
    /// The workspace version that last changed this entry.
    #[serde(default)]
    pub version: u64,
    #[serde(default)]
    pub created_ms: i64,
    #[serde(default)]
    pub updated_ms: i64,
}

fn text() -> String {
    "text".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Content {
    #[default]
    None,
    Inline {
        data: String,
    },
    /// In the object store at `blobs/{sha256}`.
    Blob {
        sha256: String,
    },
}

/// One change of a commit. Paths are canonical session paths (`/a/b.md`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Change {
    Put {
        path: String,
        encoding: String,
        content: Content,
        size: u64,
        #[serde(default)]
        readonly: bool,
        /// Only create: an existing entry is left as it is.
        #[serde(default)]
        if_absent: bool,
        /// Only replace the entry at this version (compare-and-set).
        #[serde(default)]
        expected: Option<u64>,
    },
    Delete {
        path: String,
        #[serde(default)]
        recursive: bool,
    },
    Mkdir {
        path: String,
    },
    /// Create a file that must not exist yet (an upload or the console's
    /// "new file"; a create over an existing path is refused, not merged).
    Create {
        path: String,
        encoding: String,
        content: Content,
        size: u64,
        #[serde(default)]
        readonly: bool,
    },
    /// Set or clear a path's read-only mark; its content stays.
    Mark {
        path: String,
        readonly: bool,
    },
    /// Rename a file. The content travels by reference: a blob is not
    /// copied.
    Move {
        from: String,
        to: String,
    },
    /// Copy a file; as with a move, a blob is shared, not copied.
    Copy {
        from: String,
        to: String,
    },
}

/// Why a commit did not apply.
#[derive(Debug, Clone, PartialEq)]
pub enum Refusal {
    /// A path was changed since the version the writer read.
    Conflict(String),
    ReadOnly(String),
    NotADirectory(String),
    /// The path a create, move or copy writes already exists.
    Exists(String),
    /// The path a move, copy or mark reads does not exist.
    Missing(String),
    Quota,
    Invalid(String),
}

impl Refusal {
    pub fn reply(&self) -> Value {
        match self {
            Refusal::Conflict(path) => json!({ "refused": "conflict", "path": path }),
            Refusal::ReadOnly(path) => json!({ "refused": "read_only", "path": path }),
            Refusal::NotADirectory(path) => json!({ "refused": "not_a_directory", "path": path }),
            Refusal::Exists(path) => json!({ "refused": "exists", "path": path }),
            Refusal::Missing(path) => json!({ "refused": "missing", "path": path }),
            Refusal::Quota => json!({ "refused": "quota" }),
            Refusal::Invalid(reason) => json!({ "refused": "invalid", "reason": reason }),
        }
    }
}

fn parent(path: &str) -> Option<&str> {
    let at = path.rfind('/')?;
    Some(if at == 0 { "/" } else { &path[..at] })
}

fn valid(path: &str) -> Result<(), Refusal> {
    if !path.starts_with('/') || path == "/" || path.ends_with('/') || path.contains("//") {
        return Err(Refusal::Invalid(format!("not a file path: {path}")));
    }
    if path.split('/').any(|part| part == "." || part == "..") {
        return Err(Refusal::Invalid(format!("not a canonical path: {path}")));
    }
    Ok(())
}

impl Files {
    /// The reply of an earlier commit with this operation id.
    pub fn replay(&self, op: &str) -> Option<Value> {
        self.ops
            .iter()
            .rev()
            .find(|(id, _)| id == op)
            .map(|(_, reply)| reply.clone())
    }

    fn remember(&mut self, op: String, reply: Value) {
        self.ops.push_back((op, reply));
        while self.ops.len() > KEPT_OPS {
            self.ops.pop_front();
        }
    }

    /// Apply a commit whole or not at all, and remember its reply.
    pub fn commit(&mut self, op: String, changes: Vec<Change>, now_ms: i64) -> Value {
        if let Some(reply) = self.replay(&op) {
            return reply;
        }
        let mut next = self.clone();
        next.version += 1;
        let version = next.version;
        let mut results = Vec::with_capacity(changes.len());
        let mut failed = None;
        for change in changes {
            match next.apply(change, version, now_ms) {
                Ok(result) => results.push(result),
                Err(refusal) => {
                    failed = Some(refusal);
                    break;
                }
            }
        }
        let reply = match failed {
            Some(refusal) => refusal.reply(),
            None => {
                let changed = results.iter().any(|r| r["changed"] == true);
                if changed {
                    *self = next;
                }
                json!({ "version": self.version, "results": results })
            }
        };
        self.remember(op, reply.clone());
        reply
    }

    fn mkdirs(&mut self, path: &str, version: u64, now_ms: i64) -> Result<bool, Refusal> {
        let mut changed = false;
        let mut at = String::new();
        for part in path.trim_start_matches('/').split('/') {
            at.push('/');
            at.push_str(part);
            match self.entries.get(&at) {
                Some(entry) if entry.dir => {}
                Some(_) => return Err(Refusal::NotADirectory(at)),
                None => {
                    self.entries.insert(
                        at.clone(),
                        Entry {
                            dir: true,
                            readonly: false,
                            encoding: text(),
                            content: Content::None,
                            size: 0,
                            version,
                            created_ms: now_ms,
                            updated_ms: now_ms,
                        },
                    );
                    changed = true;
                }
            }
        }
        Ok(changed)
    }

    /// The file a move or copy reads, once `to` is free for it.
    fn source(&self, from: &str, to: &str) -> Result<Entry, Refusal> {
        valid(from)?;
        valid(to)?;
        let source = self
            .entries
            .get(from)
            .cloned()
            .ok_or_else(|| Refusal::Missing(from.to_owned()))?;
        if source.dir {
            return Err(Refusal::Invalid(format!("not a file: {from}")));
        }
        if self.entries.contains_key(to) {
            return Err(Refusal::Exists(to.to_owned()));
        }
        Ok(source)
    }

    fn apply(&mut self, change: Change, version: u64, now_ms: i64) -> Result<Value, Refusal> {
        match change {
            Change::Mkdir { path } => {
                valid(&path)?;
                let changed = self.mkdirs(&path, version, now_ms)?;
                Ok(json!({ "path": path, "changed": changed }))
            }
            Change::Put {
                path,
                encoding,
                content,
                size,
                readonly,
                if_absent,
                expected,
            } => {
                valid(&path)?;
                if let Content::Inline { data } = &content
                    && data.len() > INLINE_MAX
                {
                    return Err(Refusal::Invalid(format!(
                        "inline content over {INLINE_MAX} bytes: {path}"
                    )));
                }
                let existing = self.entries.get(&path).cloned();
                if let Some(existing) = &existing {
                    if if_absent {
                        return Ok(json!({ "path": path, "changed": false, "created": false }));
                    }
                    if existing.dir {
                        return Err(Refusal::NotADirectory(path));
                    }
                    if existing.readonly {
                        return Err(Refusal::ReadOnly(path));
                    }
                    if expected.is_some_and(|v| v != existing.version) {
                        return Err(Refusal::Conflict(path));
                    }
                } else if expected.is_some() {
                    return Err(Refusal::Conflict(path));
                }
                if let Some(dir) = parent(&path).filter(|dir| *dir != "/") {
                    self.mkdirs(dir, version, now_ms)?;
                }
                let freed = existing.as_ref().map_or(0, |e| e.size);
                let bytes = self.bytes - freed + size;
                if bytes > QUOTA {
                    return Err(Refusal::Quota);
                }
                self.bytes = bytes;
                let created_ms = existing.as_ref().map_or(now_ms, |e| e.created_ms);
                self.entries.insert(
                    path.clone(),
                    Entry {
                        dir: false,
                        readonly,
                        encoding,
                        content,
                        size,
                        version,
                        created_ms,
                        updated_ms: now_ms,
                    },
                );
                Ok(json!({ "path": path, "changed": true, "created": existing.is_none() }))
            }
            Change::Create {
                path,
                encoding,
                content,
                size,
                readonly,
            } => {
                valid(&path)?;
                if self.entries.contains_key(&path) {
                    return Err(Refusal::Exists(path));
                }
                self.apply(
                    Change::Put {
                        path,
                        encoding,
                        content,
                        size,
                        readonly,
                        if_absent: false,
                        expected: None,
                    },
                    version,
                    now_ms,
                )
            }
            Change::Mark { path, readonly } => {
                valid(&path)?;
                let entry = self
                    .entries
                    .get_mut(&path)
                    .ok_or_else(|| Refusal::Missing(path.clone()))?;
                let changed = entry.readonly != readonly;
                if changed {
                    entry.readonly = readonly;
                    entry.version = version;
                    entry.updated_ms = now_ms;
                }
                Ok(json!({ "path": path, "changed": changed }))
            }
            Change::Move { from, to } | Change::Copy { from, to } if from == to => {
                valid(&from)?;
                Err(Refusal::Exists(to))
            }
            Change::Move { from, to } => {
                let source = self.source(&from, &to)?;
                if source.readonly {
                    return Err(Refusal::ReadOnly(from));
                }
                if let Some(dir) = parent(&to).filter(|dir| *dir != "/") {
                    self.mkdirs(dir, version, now_ms)?;
                }
                self.entries.remove(&from);
                self.entries.insert(
                    to.clone(),
                    Entry {
                        version,
                        updated_ms: now_ms,
                        ..source
                    },
                );
                Ok(json!({ "path": to, "changed": true }))
            }
            Change::Copy { from, to } => {
                let source = self.source(&from, &to)?;
                if let Some(dir) = parent(&to).filter(|dir| *dir != "/") {
                    self.mkdirs(dir, version, now_ms)?;
                }
                let bytes = self.bytes + source.size;
                if bytes > QUOTA {
                    return Err(Refusal::Quota);
                }
                self.bytes = bytes;
                self.entries.insert(
                    to.clone(),
                    Entry {
                        readonly: false,
                        version,
                        created_ms: now_ms,
                        updated_ms: now_ms,
                        ..source
                    },
                );
                Ok(json!({ "path": to, "changed": true }))
            }
            Change::Delete { path, recursive } => {
                valid(&path)?;
                let Some(entry) = self.entries.get(&path).cloned() else {
                    return Ok(json!({ "path": path, "changed": false }));
                };
                if entry.readonly {
                    return Ok(json!({ "path": path, "changed": false, "read_only": true }));
                }
                let prefix = format!("{path}/");
                let below: Vec<String> = self
                    .entries
                    .range(prefix.clone()..)
                    .take_while(|(p, _)| p.starts_with(&prefix))
                    .map(|(p, _)| p.clone())
                    .collect();
                if entry.dir && !below.is_empty() && !recursive {
                    return Ok(json!({ "path": path, "changed": false, "not_empty": true }));
                }
                if below.iter().any(|p| self.entries[p].readonly) {
                    return Ok(json!({ "path": path, "changed": false, "read_only": true }));
                }
                for p in below.iter().chain(std::iter::once(&path)) {
                    if let Some(gone) = self.entries.remove(p) {
                        self.bytes -= gone.size;
                    }
                }
                Ok(json!({ "path": path, "changed": true }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(path: &str, data: &str) -> Change {
        Change::Put {
            path: path.into(),
            encoding: "text".into(),
            content: Content::Inline { data: data.into() },
            size: data.len() as u64,
            readonly: false,
            if_absent: false,
            expected: None,
        }
    }

    #[test]
    fn a_commit_applies_whole_or_not_at_all() {
        let mut files = Files::default();
        files.commit("a".into(), vec![put("/x/a.md", "a")], 1);
        assert!(files.entries["/x"].dir);
        let before = files.clone();
        let reply = files.commit(
            "b".into(),
            vec![put("/x/b.md", "b"), put("/x/a.md/c", "c")],
            2,
        );
        assert_eq!(reply["refused"], "not_a_directory");
        assert_eq!(
            files.entries, before.entries,
            "nothing of a refused commit stays"
        );
        assert_eq!(files.version, before.version);
    }

    #[test]
    fn a_retried_commit_gets_its_first_reply() {
        let mut files = Files::default();
        let first = files.commit("op".into(), vec![put("/a", "1")], 1);
        files.commit("other".into(), vec![put("/a", "2")], 2);
        let again = files.commit("op".into(), vec![put("/a", "1")], 3);
        assert_eq!(first, again);
        assert_eq!(
            files.entries["/a"].content,
            Content::Inline { data: "2".into() }
        );
    }

    #[test]
    fn a_write_based_on_an_old_version_conflicts() {
        let mut files = Files::default();
        files.commit("1".into(), vec![put("/a", "1")], 1);
        let read = files.entries["/a"].version;
        files.commit("2".into(), vec![put("/a", "2")], 2);
        let mut stale = put("/a", "3");
        if let Change::Put { expected, .. } = &mut stale {
            *expected = Some(read);
        }
        assert_eq!(
            files.commit("3".into(), vec![stale], 3)["refused"],
            "conflict"
        );
    }

    #[test]
    fn read_only_entries_stay() {
        let mut files = Files::default();
        files.commit(
            "seed".into(),
            vec![Change::Put {
                path: "/h/index.md".into(),
                encoding: "text".into(),
                content: Content::Inline { data: "#".into() },
                size: 1,
                readonly: true,
                if_absent: true,
                expected: None,
            }],
            1,
        );
        assert_eq!(
            files.commit("w".into(), vec![put("/h/index.md", "x")], 2)["refused"],
            "read_only"
        );
        let gone = files.commit(
            "d".into(),
            vec![Change::Delete {
                path: "/h".into(),
                recursive: true,
            }],
            3,
        );
        assert_eq!(gone["results"][0]["changed"], false);
        assert!(files.entries.contains_key("/h/index.md"));
    }

    #[test]
    fn the_quota_holds() {
        let mut files = Files::default();
        let big = Change::Put {
            path: "/big".into(),
            encoding: "base64".into(),
            content: Content::Blob {
                sha256: "00".into(),
            },
            size: QUOTA + 1,
            readonly: false,
            if_absent: false,
            expected: None,
        };
        assert_eq!(files.commit("q".into(), vec![big], 1)["refused"], "quota");
        assert_eq!(files.bytes, 0);
    }

    #[test]
    fn deleting_a_directory_frees_its_bytes() {
        let mut files = Files::default();
        files.commit("1".into(), vec![put("/d/a", "aaa"), put("/d/e/b", "bb")], 1);
        assert_eq!(files.bytes, 5);
        let reply = files.commit(
            "2".into(),
            vec![Change::Delete {
                path: "/d".into(),
                recursive: false,
            }],
            2,
        );
        assert_eq!(reply["results"][0]["not_empty"], true);
        files.commit(
            "3".into(),
            vec![Change::Delete {
                path: "/d".into(),
                recursive: true,
            }],
            3,
        );
        assert!(files.entries.is_empty());
        assert_eq!(files.bytes, 0);
    }

    #[test]
    fn create_move_and_copy_refuse_an_existing_target() {
        let mut files = Files::default();
        files.commit("1".into(), vec![put("/a.md", "a"), put("/b.md", "b")], 1);
        let create = |path: &str| Change::Create {
            path: path.into(),
            encoding: "text".into(),
            content: Content::Inline { data: "c".into() },
            size: 1,
            readonly: false,
        };
        assert_eq!(
            files.commit("2".into(), vec![create("/a.md")], 2)["refused"],
            "exists"
        );
        let moved = Change::Move {
            from: "/a.md".into(),
            to: "/b.md".into(),
        };
        assert_eq!(
            files.commit("3".into(), vec![moved], 3)["refused"],
            "exists"
        );
        let copied = Change::Copy {
            from: "/gone.md".into(),
            to: "/c.md".into(),
        };
        assert_eq!(
            files.commit("4".into(), vec![copied], 4)["refused"],
            "missing"
        );
        assert_eq!(files.entries.len(), 2);
    }

    #[test]
    fn a_move_keeps_the_content_and_a_copy_counts_its_bytes() {
        let mut files = Files::default();
        files.commit("1".into(), vec![put("/a.md", "abc")], 1);
        files.commit(
            "2".into(),
            vec![Change::Move {
                from: "/a.md".into(),
                to: "/x/b.md".into(),
            }],
            2,
        );
        assert!(!files.entries.contains_key("/a.md"));
        assert!(files.entries["/x"].dir);
        assert_eq!(
            files.entries["/x/b.md"].content,
            Content::Inline { data: "abc".into() }
        );
        assert_eq!(files.bytes, 3);
        files.commit(
            "3".into(),
            vec![Change::Copy {
                from: "/x/b.md".into(),
                to: "/c.md".into(),
            }],
            3,
        );
        assert_eq!(files.bytes, 6);
        assert_eq!(
            files.entries["/c.md"].content,
            files.entries["/x/b.md"].content
        );
    }

    #[test]
    fn a_read_only_file_can_be_unmarked_but_not_moved() {
        let mut files = Files::default();
        files.commit("1".into(), vec![put("/a.md", "a")], 1);
        let mark = |readonly| Change::Mark {
            path: "/a.md".into(),
            readonly,
        };
        files.commit("2".into(), vec![mark(true)], 2);
        assert!(files.entries["/a.md"].readonly);
        let moved = Change::Move {
            from: "/a.md".into(),
            to: "/b.md".into(),
        };
        assert_eq!(
            files.commit("3".into(), vec![moved.clone()], 3)["refused"],
            "read_only"
        );
        files.commit("4".into(), vec![mark(false)], 4);
        assert_eq!(
            files.commit("5".into(), vec![moved], 5)["results"][0]["changed"],
            true
        );
        assert_eq!(
            files.commit("6".into(), vec![mark(true)], 6)["refused"],
            "missing"
        );
    }
}
