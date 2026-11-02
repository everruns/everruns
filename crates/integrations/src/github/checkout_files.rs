//! `github_checkout_pull_request` without a managed sandbox: copy the pull
//! request's head into the session's files, where the Bashkit shell and the
//! file tools read it.
//!
//! Decision: Bashkit is a virtual shell with no `git clone`, so the head is
//! fetched host-side as one GitHub tarball and unpacked into
//! `/workspace/<repo>`. The token never reaches the shell. What lands is a
//! snapshot of text files, not a repository: no history, no `.git`, binaries
//! and files over the per-file limit are skipped and reported, and the whole
//! copy is capped so a large repository cannot flood the session's storage.
//! Reviewers get the change itself from `get_github_pull_request_diff`.

use std::io::Read;
use std::sync::Arc;

use everruns_contracts::runtime::session_files::SessionFileSystem;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::ToolExecutionResult;
use futures::StreamExt;
use serde_json::{Value, json};

use crate::github::client::GitHubClient;

/// Compressed archive ceiling; GitHub tarballs of big monorepos exceed this.
const MAX_ARCHIVE_BYTES: usize = 256 * 1024 * 1024;
/// Larger files are usually generated reports, fixtures or vendored blobs.
/// 256 KiB proved too small: bashkit's Python binding `lib.rs` and its threat
/// model are over 500 KiB, and those are exactly what a reviewer reads.
pub(super) const DEFAULT_MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_FILES: usize = 10_000;
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
/// Session file writes are independent; a few in flight hide store latency.
const WRITE_CONCURRENCY: usize = 16;
/// Names listed per skip reason, so the model can fetch one it needs.
const LISTED_SKIPS: usize = 20;

#[derive(Debug, Default)]
pub(super) struct Snapshot {
    pub files: Vec<(String, String)>,
    pub bytes: u64,
    pub too_large: Vec<String>,
    pub too_large_count: usize,
    pub binary_count: usize,
    pub filtered_count: usize,
    /// The file or byte cap was reached; later files are missing.
    pub truncated: bool,
}

fn safe_relative(path: &std::path::Path) -> Option<String> {
    let mut parts = Vec::new();
    // The first component is GitHub's `<owner>-<repo>-<sha>/` wrapper.
    for (index, component) in path.components().enumerate() {
        let std::path::Component::Normal(part) = component else {
            return None;
        };
        if index == 0 {
            continue;
        }
        let part = part.to_str()?;
        if part.is_empty() || part.contains('\\') {
            return None;
        }
        parts.push(part);
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn selected(relative: &str, paths: &[String]) -> bool {
    paths.is_empty()
        || paths.iter().any(|prefix| {
            relative == prefix
                || relative
                    .strip_prefix(prefix.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
        })
}

/// Unpack the text files of a GitHub tarball, within the limits.
pub(super) fn extract(
    archive: &[u8],
    paths: &[String],
    max_file_bytes: u64,
) -> Result<Snapshot, String> {
    let mut snapshot = Snapshot::default();
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let entries = tar
        .entries()
        .map_err(|e| format!("Invalid repository archive: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("Invalid repository archive: {e}"))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let Some(relative) = entry.path().ok().and_then(|path| safe_relative(&path)) else {
            continue;
        };
        if !selected(&relative, paths) {
            snapshot.filtered_count += 1;
            continue;
        }
        let size = entry.header().size().unwrap_or(u64::MAX);
        if size > max_file_bytes {
            snapshot.too_large_count += 1;
            if snapshot.too_large.len() < LISTED_SKIPS {
                snapshot.too_large.push(relative);
            }
            continue;
        }
        if snapshot.files.len() >= MAX_FILES || snapshot.bytes + size > MAX_TOTAL_BYTES {
            snapshot.truncated = true;
            break;
        }
        let mut content = Vec::with_capacity(size as usize);
        entry
            .read_to_end(&mut content)
            .map_err(|e| format!("Invalid repository archive: {e}"))?;
        match String::from_utf8(content) {
            Ok(text) if !text.contains('\0') => {
                snapshot.bytes += size;
                snapshot.files.push((relative, text));
            }
            _ => snapshot.binary_count += 1,
        }
    }
    Ok(snapshot)
}

/// Write the snapshot under `root`. What it covers is replaced first, so files
/// the new head deleted do not linger: the whole checkout, or only the
/// requested `paths`, so a narrow follow-up call adds to an earlier full copy
/// instead of wiping it.
async fn write(
    store: &Arc<dyn SessionFileSystem>,
    context: &ToolContext,
    root: &str,
    paths: &[String],
    files: Vec<(String, String)>,
) -> (usize, usize, Vec<String>) {
    // Absent on a first checkout; any real failure shows up on the writes.
    if paths.is_empty() {
        let _ = store.delete_file(context.session_id, root, true).await;
    } else {
        for path in paths {
            let _ = store
                .delete_file(context.session_id, &format!("{root}/{path}"), true)
                .await;
        }
    }
    let results: Vec<Result<(), String>> = futures::stream::iter(files)
        .map(|(relative, content)| {
            let store = store.clone();
            let path = format!("{root}/{relative}");
            async move {
                store
                    .write_file(context.session_id, &path, &content, "text")
                    .await
                    .map(|_| ())
                    .map_err(|e| format!("{relative}: {e}"))
            }
        })
        .buffer_unordered(WRITE_CONCURRENCY)
        .collect()
        .await;
    let mut written = 0;
    let mut failed_count = 0;
    let mut failed = Vec::new();
    for result in results {
        match result {
            Ok(()) => written += 1,
            Err(e) => {
                failed_count += 1;
                if failed.len() < LISTED_SKIPS {
                    failed.push(e);
                }
            }
        }
    }
    (written, failed_count, failed)
}

pub(super) struct FileCheckout<'a> {
    pub repo: &'a str,
    pub number: u64,
    pub path: &'a str,
    pub paths: &'a [String],
    pub max_file_bytes: u64,
}

pub(super) async fn checkout_into_files(
    client: &GitHubClient,
    store: &Arc<dyn SessionFileSystem>,
    context: &ToolContext,
    request: FileCheckout<'_>,
) -> Result<Value, ToolExecutionResult> {
    let pull = client
        .pull_request(request.repo, request.number)
        .await
        .map_err(ToolExecutionResult::tool_error)?;
    let archive = client
        .tarball(request.repo, &pull.head.sha, MAX_ARCHIVE_BYTES)
        .await
        .map_err(ToolExecutionResult::tool_error)?;
    let paths = request.paths.to_vec();
    let max_file_bytes = request.max_file_bytes;
    let snapshot = tokio::task::spawn_blocking(move || extract(&archive, &paths, max_file_bytes))
        .await
        .map_err(|e| ToolExecutionResult::internal_error_msg(format!("unpack failed: {e}")))?
        .map_err(ToolExecutionResult::tool_error)?;
    let Snapshot {
        files,
        bytes,
        too_large,
        too_large_count,
        binary_count,
        filtered_count,
        truncated,
    } = snapshot;
    let (written, failed_count, failed) =
        write(store, context, request.path, request.paths, files).await;
    if written == 0 && !failed.is_empty() {
        return Err(ToolExecutionResult::tool_error(format!(
            "Could not write the checkout into session files: {}",
            failed.join("; ")
        )));
    }
    let base_sha = &pull.base.sha;
    Ok(json!({
        "repo": request.repo,
        "number": request.number,
        "path": request.path,
        "mode": "files",
        "head_sha": pull.head.sha,
        "head_ref": pull.head.name,
        "base_ref": pull.base.name,
        "base_sha": base_sha,
        "files_written": written,
        "bytes_written": bytes,
        "skipped": {
            "too_large": too_large_count,
            "too_large_examples": too_large,
            "binary": binary_count,
            "outside_paths": filtered_count,
            "write_failed": failed_count,
            "write_failed_examples": failed,
        },
        "truncated": truncated,
        "hint": format!(
            "Copied the pull request head's text files into {}. This is a snapshot, not a git \
             repository: there is no history, so get the change itself with \
             get_github_pull_request_diff (base {base_sha}). Read and search the files with \
             bash (grep, rg, find, cat) or the file tools. Call this again after a new push.",
            request.path
        ),
    }))
}

/// A GitHub-shaped tarball (one wrapper directory) for tests.
#[cfg(test)]
pub(super) fn test_archive(files: &[(&str, &[u8])]) -> Vec<u8> {
    tests::archive(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        for (path, content) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, format!("acme-app-1111/{path}"), *content)
                .unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn extract_keeps_text_and_reports_what_it_skipped() {
        let big = vec![b'a'; 64];
        let data = archive(&[
            ("AGENTS.md", b"# rules\n"),
            ("src/lib.rs", b"fn main() {}\n"),
            ("assets/logo.png", b"\x89PNG\0\0"),
            ("report.html", &big),
        ]);
        let snapshot = extract(&data, &[], 32).unwrap();
        let names: Vec<&str> = snapshot.files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(names, ["AGENTS.md", "src/lib.rs"]);
        assert_eq!(snapshot.binary_count, 1);
        assert_eq!(snapshot.too_large, ["report.html"]);
        assert!(!snapshot.truncated);
    }

    #[test]
    fn extract_limits_to_requested_paths() {
        let data = archive(&[
            ("AGENTS.md", b"x"),
            ("crates/a/lib.rs", b"x"),
            ("crates/ab/lib.rs", b"x"),
        ]);
        let snapshot = extract(&data, &["crates/a".into(), "AGENTS.md".into()], 1024).unwrap();
        let names: Vec<&str> = snapshot.files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(names, ["AGENTS.md", "crates/a/lib.rs"]);
        assert_eq!(snapshot.filtered_count, 1);
    }

    #[test]
    fn safe_relative_rejects_escapes() {
        use std::path::Path;
        assert_eq!(
            safe_relative(Path::new("w/a/b.rs")).as_deref(),
            Some("a/b.rs")
        );
        assert_eq!(safe_relative(Path::new("w/../etc/passwd")), None);
        assert_eq!(safe_relative(Path::new("/etc/passwd")), None);
        assert_eq!(safe_relative(Path::new("w")), None);
    }
}
