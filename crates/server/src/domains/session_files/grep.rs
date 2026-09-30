// Regex search over a session's files, and the TM-DOS-008 bounds that keep it
// affordable: pattern length, compiled automaton size, per-file bytes, and
// total bytes scanned in one call.
//
// Split out of `service.rs`, which is on the source-file size debt list.

use anyhow::{Result, anyhow};
use regex::{Regex, RegexBuilder};
use uuid::Uuid;

use super::service::WorkspaceFileService;
use super::virtual_mount_registry::VirtualMountRegistry;
use crate::kernel_imports::{
    GrepMatch, GrepOptions, GrepResult, GrepSearchResult, session_file::build_grep_search_result,
};
use crate::storage::StorageBackend;

impl WorkspaceFileService {
    /// Search files with bounded result metadata, excluding a private subtree
    /// before content matching and accounting (THREAT[TM-TENANT-013]).
    pub async fn grep_with_options_excluding(
        &self,
        session_id: Uuid,
        pattern: &str,
        options: &GrepOptions,
        excluded_path_prefix: Option<&str>,
    ) -> Result<GrepSearchResult> {
        grep_session_files_with_options(
            &self.db,
            self.virtual_registry.as_deref(),
            session_id,
            pattern,
            options,
            excluded_path_prefix,
        )
        .await
    }
}

/// Max regex pattern length (TM-DOS-008): bounds compilation cost even before NFA construction.
pub(super) const MAX_GREP_PATTERN_LEN: usize = 1000;
/// Max NFA/DFA compiled size in bytes (TM-DOS-008): prevents short patterns that expand to
/// enormous automata (e.g. deeply nested alternation).
pub(super) const MAX_GREP_REGEX_SIZE: usize = 512 * 1024;
/// Max file size to search (TM-DOS-008): skip files larger than this to bound per-file scan time.
pub(super) const MAX_GREP_FILE_BYTES: i64 = 512 * 1024;
/// Max total bytes scanned across all files in a single grep call (TM-DOS-008).
pub(super) const MAX_GREP_TOTAL_SCAN_BYTES: usize = 5 * 1024 * 1024;

/// Build a regex with compile-time size limits applied (TM-DOS-008).
///
/// The `regex` crate uses a Thompson NFA and cannot catastrophically backtrack,
/// but a short pattern can still compile to a very large automaton. `size_limit`
/// caps that at `MAX_GREP_REGEX_SIZE` bytes.
pub(super) fn build_grep_regex(pattern: &str) -> Result<Regex> {
    RegexBuilder::new(pattern)
        .size_limit(MAX_GREP_REGEX_SIZE)
        .build()
        .map_err(|e| anyhow!("Invalid or too-complex regex pattern: {e}"))
}

/// Search session files using grep-like regex pattern matching.
///
/// Shared logic used by both `WorkspaceFileService::grep` and
/// `DirectWorkerAdapters::grep_files`. Enforces TM-DOS-008 bounds.
pub async fn grep_session_files(
    db: &StorageBackend,
    session_id: Uuid,
    pattern: &str,
    path_pattern: Option<&str>,
) -> Result<Vec<GrepResult>> {
    grep_session_files_excluding(db, session_id, pattern, path_pattern, None).await
}

pub(super) async fn grep_session_files_excluding(
    db: &StorageBackend,
    session_id: Uuid,
    pattern: &str,
    path_pattern: Option<&str>,
    excluded_path_prefix: Option<&str>,
) -> Result<Vec<GrepResult>> {
    // TM-DOS-008: cap content/path pattern lengths, then cap content-regex NFA size.
    anyhow::ensure!(
        pattern.len() <= MAX_GREP_PATTERN_LEN,
        "Regex pattern too long (max {} characters)",
        MAX_GREP_PATTERN_LEN
    );
    if let Some(pp) = path_pattern {
        anyhow::ensure!(
            pp.len() <= MAX_GREP_PATTERN_LEN,
            "Path pattern too long (max {} characters)",
            MAX_GREP_PATTERN_LEN
        );
    }

    let regex = build_grep_regex(pattern)?;
    let path_matcher = path_pattern
        .map(everruns_core::session_path::GrepPathPattern::new)
        .transpose()?;

    // A present path filter tells storage to return metadata candidates without
    // scanning content. The shared matcher below then narrows those candidates
    // before content is fetched and charged to the total scan budget.
    let files = db
        .grep_session_files(
            session_id,
            pattern,
            path_pattern,
            excluded_path_prefix,
            MAX_GREP_FILE_BYTES,
        )
        .await?;

    let mut results = Vec::new();
    let mut total_scanned: usize = 0;

    // For each matching file, find the actual line matches
    for file_info in files {
        if path_matcher
            .as_ref()
            .is_some_and(|matcher| !matcher.is_match(&file_info.path))
        {
            continue;
        }
        // Defense-in-depth: skip oversized files even if the storage filter missed them.
        if file_info.size_bytes > MAX_GREP_FILE_BYTES {
            continue;
        }

        // TM-DOS-008: abort if total bytes scanned across all files exceeds the cap.
        let file_size = file_info.size_bytes.max(0) as usize;
        anyhow::ensure!(
            total_scanned.saturating_add(file_size) <= MAX_GREP_TOTAL_SCAN_BYTES,
            "Grep request exceeds maximum scan size ({} bytes); narrow the path filter or pattern",
            MAX_GREP_TOTAL_SCAN_BYTES
        );
        total_scanned += file_size;

        // Read full file content
        let file = db.get_session_file(session_id, &file_info.path).await?;
        if let Some(f) = file
            && let Some(content) = f.content
            && let Ok(text) = String::from_utf8(content)
        {
            let matches: Vec<GrepMatch> = text
                .lines()
                .enumerate()
                .filter(|(_, line)| regex.is_match(line))
                .map(|(i, line)| GrepMatch {
                    path: file_info.path.clone(),
                    line_number: i + 1,
                    line: line.to_string(),
                })
                .collect();

            if !matches.is_empty() {
                results.push(GrepResult {
                    path: file_info.path.clone(),
                    matches,
                });
            }
        }
    }

    Ok(results)
}

pub(crate) async fn grep_session_files_with_options(
    db: &StorageBackend,
    virtual_registry: Option<&VirtualMountRegistry>,
    session_id: Uuid,
    pattern: &str,
    options: &GrepOptions,
    excluded_path_prefix: Option<&str>,
) -> Result<GrepSearchResult> {
    anyhow::ensure!(
        pattern.len() <= MAX_GREP_PATTERN_LEN,
        "Regex pattern too long (max {} characters)",
        MAX_GREP_PATTERN_LEN
    );
    if let Some(path_pattern) = options.path_pattern.as_deref() {
        anyhow::ensure!(
            path_pattern.len() <= MAX_GREP_PATTERN_LEN,
            "Path pattern too long (max {} characters)",
            MAX_GREP_PATTERN_LEN
        );
    }
    anyhow::ensure!(
        options.before_context <= everruns_core::GREP_MAX_CONTEXT_LINES,
        "before_context exceeds maximum of {}",
        everruns_core::GREP_MAX_CONTEXT_LINES
    );
    anyhow::ensure!(
        options.after_context <= everruns_core::GREP_MAX_CONTEXT_LINES,
        "after_context exceeds maximum of {}",
        everruns_core::GREP_MAX_CONTEXT_LINES
    );
    let regex = build_grep_regex(pattern)?;
    let path_matcher = options
        .path_pattern
        .as_deref()
        .map(everruns_core::session_path::GrepPathPattern::new)
        .transpose()?;
    let rows = db
        .grep_session_files(
            session_id,
            pattern,
            options.path_pattern.as_deref(),
            excluded_path_prefix,
            MAX_GREP_FILE_BYTES,
        )
        .await?;
    let mut text_files = Vec::new();
    let mut total_scanned = 0usize;
    for row in rows {
        if excluded_path_prefix
            .is_some_and(|prefix| row.path == prefix || row.path.starts_with(&format!("{prefix}/")))
            || path_matcher
                .as_ref()
                .is_some_and(|matcher| !matcher.is_match(&row.path))
            || row.size_bytes > MAX_GREP_FILE_BYTES
        {
            continue;
        }
        total_scanned = total_scanned.saturating_add(row.size_bytes.max(0) as usize);
        anyhow::ensure!(
            total_scanned <= MAX_GREP_TOTAL_SCAN_BYTES,
            "Grep request exceeds maximum scan size ({} bytes); narrow the path filter or pattern",
            MAX_GREP_TOTAL_SCAN_BYTES
        );
        if let Some(file) = db.get_session_file(session_id, &row.path).await?
            && let Some(content) = file.content
            && let Ok(text) = String::from_utf8(content)
        {
            text_files.push((row.path, text));
        }
    }
    if let Some(registry) = virtual_registry {
        for (path, text) in registry.grep_text_files(&session_id, MAX_GREP_FILE_BYTES as usize) {
            if !excluded_path_prefix
                .is_some_and(|prefix| path == prefix || path.starts_with(&format!("{prefix}/")))
                && path_matcher
                    .as_ref()
                    .is_none_or(|matcher| matcher.is_match(&path))
            {
                total_scanned = total_scanned.saturating_add(text.len());
                anyhow::ensure!(
                    total_scanned <= MAX_GREP_TOTAL_SCAN_BYTES,
                    "Grep request exceeds maximum scan size ({} bytes); narrow the path filter or pattern",
                    MAX_GREP_TOTAL_SCAN_BYTES
                );
                text_files.push((path, text));
            }
        }
    }
    Ok(build_grep_search_result(text_files, &regex, options))
}
