// Session File domain types (Virtual Filesystem)
//
// These types represent files and directories stored within a session's
// virtual filesystem. Each session has its own isolated filesystem.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Maximum number of context lines accepted on either side of a grep match.
pub const GREP_MAX_CONTEXT_LINES: usize = 20;
/// Maximum serialized entry bytes returned by one grep request.
pub const GREP_MAX_RETURN_BYTES: usize = 64 * 1024;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// File metadata without content
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct FileInfo {
    /// Internal database UUID for this file entry.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "550e8400-e29b-41d4-a716-446655440000")
    )]
    pub id: Uuid,
    /// UUID of the owning session.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "01933b5a-0000-7000-8000-000000000001")
    )]
    pub session_id: Uuid,
    /// Absolute path within the session workspace (e.g. `/notes.md`).
    #[cfg_attr(feature = "openapi", schema(example = "/notes.md"))]
    pub path: String,
    /// File or directory name (the last segment of `path`).
    #[cfg_attr(feature = "openapi", schema(example = "notes.md"))]
    pub name: String,
    /// `true` when this entry represents a directory; `false` for a regular file.
    #[cfg_attr(feature = "openapi", schema(example = false))]
    pub is_directory: bool,
    /// Whether the entry was marked read-only at creation. Read-only entries cannot be edited or deleted by the session.
    #[cfg_attr(feature = "openapi", schema(example = false))]
    pub is_readonly: bool,
    /// File size in bytes. `0` for directories.
    #[cfg_attr(feature = "openapi", schema(example = 4096))]
    pub size_bytes: i64,
    /// Timestamp when this entry was created (RFC 3339).
    #[cfg_attr(feature = "openapi", schema(example = "2026-05-25T10:14:00Z"))]
    pub created_at: DateTime<Utc>,
    /// Timestamp when this entry was last updated (RFC 3339).
    #[cfg_attr(feature = "openapi", schema(example = "2026-05-25T10:15:30Z"))]
    pub updated_at: DateTime<Utc>,
}

impl FileInfo {
    /// Extract file name from path
    pub fn name_from_path(path: &str) -> String {
        if path == "/" {
            "/".to_string()
        } else {
            path.rsplit('/').next().unwrap_or(path).to_string()
        }
    }

    /// Get parent directory path
    pub fn parent_path(path: &str) -> Option<String> {
        if path == "/" {
            None
        } else {
            let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("/");
            Some(if parent.is_empty() { "/" } else { parent }.to_string())
        }
    }
}

/// Complete file with content
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SessionFile {
    /// Internal database UUID for this file entry.
    pub id: Uuid,
    /// UUID of the owning session.
    pub session_id: Uuid,
    /// Absolute path within the session workspace (e.g. `/notes.md`).
    pub path: String,
    /// File or directory name (the last segment of `path`).
    pub name: String,
    /// File content. Encoding is controlled by the `encoding` field: plain UTF-8 text for `text`, base64-encoded bytes for `base64`. `None` for directories and when this is a metadata-only listing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Content encoding for the `content` field: `text` (UTF-8) or `base64` (binary).
    #[serde(default = "default_encoding")]
    pub encoding: String,
    /// `true` when this entry represents a directory; `false` for a regular file.
    pub is_directory: bool,
    /// Whether the entry was marked read-only at creation. Read-only entries cannot be edited or deleted by the session.
    pub is_readonly: bool,
    /// File size in bytes. `0` for directories.
    pub size_bytes: i64,
    /// Timestamp when this entry was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this entry was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
}

/// Starter file copied into a new session from an agent or harness.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct InitialFile {
    /// Absolute path within the session workspace. `/workspace` prefix is accepted.
    pub path: String,
    /// File content: plain text or base64-encoded binary.
    pub content: String,
    /// Content encoding: `text` or `base64`.
    #[serde(default = "default_encoding")]
    pub encoding: String,
    /// Prevent session-side edits or deletes when true.
    #[serde(default)]
    pub is_readonly: bool,
}

fn default_encoding() -> String {
    "text".to_string()
}

impl SessionFile {
    /// Check if content is likely text based on bytes
    pub fn is_text_content(bytes: &[u8]) -> bool {
        // Quick heuristic: check first 8KB for null bytes
        let check_len = bytes.len().min(8192);
        !bytes[..check_len].contains(&0)
    }

    /// Convert raw bytes to content string with appropriate encoding
    pub fn encode_content(bytes: &[u8]) -> (String, String) {
        if Self::is_text_content(bytes) {
            match String::from_utf8(bytes.to_vec()) {
                Ok(text) => (text, "text".to_string()),
                Err(_) => (BASE64.encode(bytes), "base64".to_string()),
            }
        } else {
            (BASE64.encode(bytes), "base64".to_string())
        }
    }

    /// Decode content string to raw bytes
    pub fn decode_content(content: &str, encoding: &str) -> Result<Vec<u8>, base64::DecodeError> {
        match encoding {
            "base64" => BASE64.decode(content),
            _ => Ok(content.as_bytes().to_vec()),
        }
    }
}

/// File stat information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct FileStat {
    /// Absolute path within the session workspace.
    pub path: String,
    /// File or directory name (last segment of `path`).
    pub name: String,
    /// `true` when this entry represents a directory.
    pub is_directory: bool,
    /// Whether the entry is read-only.
    pub is_readonly: bool,
    /// File size in bytes. `0` for directories.
    pub size_bytes: i64,
    /// Timestamp when this entry was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this entry was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
}

/// Grep match result
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GrepMatch {
    pub path: String,
    pub line_number: usize,
    pub line: String,
}

/// Options for a bounded grep scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrepOptions {
    pub path_pattern: Option<String>,
    pub before_context: usize,
    pub after_context: usize,
    pub offset: usize,
    pub limit: usize,
    pub max_bytes: usize,
}

impl Default for GrepOptions {
    fn default() -> Self {
        Self {
            path_pattern: None,
            before_context: 0,
            after_context: 0,
            offset: 0,
            limit: usize::MAX,
            max_bytes: GREP_MAX_RETURN_BYTES,
        }
    }
}

/// One numbered line in a contextual grep block.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GrepContextLine {
    pub line_number: usize,
    pub line: String,
    pub is_match: bool,
}

/// A contiguous contextual range. Overlapping match windows are merged.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GrepContextBlock {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub match_line_numbers: Vec<usize>,
    pub lines: Vec<GrepContextLine>,
}

/// Backend result for a bounded grep scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GrepSearchResult {
    /// Flat matches are populated when both context values are zero.
    pub matches: Vec<GrepMatch>,
    /// Context blocks are populated when either context value is non-zero.
    pub blocks: Vec<GrepContextBlock>,
    pub total_matches: usize,
    pub returned_matches: usize,
    pub bytes_returned: usize,
    pub bytes_total: usize,
    pub next_offset: Option<usize>,
    pub byte_truncated: bool,
}

/// Build a bounded result from text files already loaded by a backend scan.
/// Paths are sorted so match offsets are stable across backend implementations.
pub fn build_grep_search_result(
    mut files: Vec<(String, String)>,
    regex: &regex::Regex,
    options: &GrepOptions,
) -> GrepSearchResult {
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut total_matches = 0usize;
    let mut remaining_offset = options.offset;
    let mut remaining_limit = options.limit;
    let mut flat = Vec::new();
    let mut blocks = Vec::new();

    for (path, text) in files {
        let lines: Vec<&str> = text.lines().collect();
        let file_matches: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| regex.is_match(line).then_some(index))
            .collect();
        total_matches = total_matches.saturating_add(file_matches.len());

        let skip = remaining_offset.min(file_matches.len());
        remaining_offset -= skip;
        let selected: Vec<usize> = file_matches
            .into_iter()
            .skip(skip)
            .take(remaining_limit)
            .collect();
        remaining_limit = remaining_limit.saturating_sub(selected.len());

        if options.before_context == 0 && options.after_context == 0 {
            flat.extend(selected.into_iter().map(|index| GrepMatch {
                path: path.clone(),
                line_number: index + 1,
                line: lines[index].to_string(),
            }));
            continue;
        }

        let mut ranges: Vec<(usize, usize, Vec<usize>)> = Vec::new();
        for index in selected {
            let start = index.saturating_sub(options.before_context);
            let end = index
                .saturating_add(options.after_context)
                .min(lines.len().saturating_sub(1));
            if let Some((_, previous_end, match_indexes)) = ranges.last_mut()
                && start <= previous_end.saturating_add(1)
            {
                *previous_end = (*previous_end).max(end);
                match_indexes.push(index);
            } else {
                ranges.push((start, end, vec![index]));
            }
        }

        for (start, end, match_indexes) in ranges {
            let context_lines = (start..=end)
                .map(|index| GrepContextLine {
                    line_number: index + 1,
                    line: lines[index].to_string(),
                    is_match: match_indexes.binary_search(&index).is_ok(),
                })
                .collect();
            blocks.push(GrepContextBlock {
                path: path.clone(),
                start_line: start + 1,
                end_line: end + 1,
                match_line_numbers: match_indexes.into_iter().map(|index| index + 1).collect(),
                lines: context_lines,
            });
        }
    }

    apply_grep_byte_budget(flat, blocks, total_matches, options)
}

/// Apply stable match pagination and the response byte budget to flat matches.
pub fn bound_grep_matches(mut matches: Vec<GrepMatch>, options: &GrepOptions) -> GrepSearchResult {
    matches.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.line_number.cmp(&b.line_number))
            .then(a.line.cmp(&b.line))
    });
    let total_matches = matches.len();
    let selected = matches
        .into_iter()
        .skip(options.offset)
        .take(options.limit)
        .collect();
    apply_grep_byte_budget(selected, Vec::new(), total_matches, options)
}

/// Merge results from distinct mounts, then apply one global match window.
pub fn merge_grep_search_results(
    results: Vec<GrepSearchResult>,
    options: &GrepOptions,
) -> GrepSearchResult {
    if options.before_context == 0 && options.after_context == 0 {
        return bound_grep_matches(
            results
                .into_iter()
                .flat_map(|result| result.matches)
                .collect(),
            options,
        );
    }

    let mut lines_by_path: BTreeMap<String, BTreeMap<usize, String>> = BTreeMap::new();
    let mut matches_by_path: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    for result in results {
        for block in result.blocks {
            let path_lines = lines_by_path.entry(block.path.clone()).or_default();
            for line in block.lines {
                path_lines.entry(line.line_number).or_insert(line.line);
            }
            matches_by_path
                .entry(block.path)
                .or_default()
                .extend(block.match_line_numbers);
        }
    }

    let total_matches = matches_by_path.values().map(BTreeSet::len).sum();
    let selected: Vec<(String, usize)> = matches_by_path
        .iter()
        .flat_map(|(path, lines)| lines.iter().map(move |line| (path.clone(), *line)))
        .skip(options.offset)
        .take(options.limit)
        .collect();
    let mut selected_by_path: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (path, line) in selected {
        selected_by_path.entry(path).or_default().push(line);
    }

    let mut blocks = Vec::new();
    for (path, match_lines) in selected_by_path {
        let available = &lines_by_path[&path];
        let mut ranges: Vec<(usize, usize, Vec<usize>)> = Vec::new();
        for line in match_lines {
            let start = line.saturating_sub(options.before_context).max(1);
            let end = line.saturating_add(options.after_context);
            if let Some((_, previous_end, matches)) = ranges.last_mut()
                && start <= previous_end.saturating_add(1)
            {
                *previous_end = (*previous_end).max(end);
                matches.push(line);
            } else {
                ranges.push((start, end, vec![line]));
            }
        }
        for (start, end, match_line_numbers) in ranges {
            let selected_set: BTreeSet<_> = match_line_numbers.iter().copied().collect();
            let lines: Vec<_> = available
                .range(start..=end)
                .map(|(line_number, line)| GrepContextLine {
                    line_number: *line_number,
                    line: line.clone(),
                    is_match: selected_set.contains(line_number),
                })
                .collect();
            if let (Some(first), Some(last)) = (lines.first(), lines.last()) {
                blocks.push(GrepContextBlock {
                    path: path.clone(),
                    start_line: first.line_number,
                    end_line: last.line_number,
                    match_line_numbers,
                    lines,
                });
            }
        }
    }
    apply_grep_byte_budget(Vec::new(), blocks, total_matches, options)
}

fn apply_grep_byte_budget(
    flat: Vec<GrepMatch>,
    blocks: Vec<GrepContextBlock>,
    total_matches: usize,
    options: &GrepOptions,
) -> GrepSearchResult {
    let bytes_total = flat.iter().map(serialized_entry_len).sum::<usize>()
        + blocks.iter().map(serialized_entry_len).sum::<usize>();
    let mut bytes_returned = 0usize;
    let mut returned_matches = 0usize;
    let mut byte_truncated = false;
    let mut returned_flat = Vec::new();
    let mut returned_blocks = Vec::new();

    for mut item in flat {
        let remaining = options.max_bytes.saturating_sub(bytes_returned);
        let mut item_bytes = serialized_entry_len(&item);
        if item_bytes > remaining {
            if !returned_flat.is_empty() || remaining == 0 {
                byte_truncated = true;
                break;
            }
            truncate_line_to_serialized_size(&mut item, remaining);
            item_bytes = serialized_entry_len(&item);
            byte_truncated = true;
            if item_bytes > remaining {
                break;
            }
        }
        bytes_returned += item_bytes;
        returned_matches += 1;
        returned_flat.push(item);
        if byte_truncated {
            break;
        }
    }

    for mut block in blocks {
        let remaining = options.max_bytes.saturating_sub(bytes_returned);
        let mut block_bytes = serialized_entry_len(&block);
        if block_bytes > remaining {
            if !returned_blocks.is_empty() || remaining == 0 {
                byte_truncated = true;
                break;
            }
            truncate_block_to_serialized_size(&mut block, remaining);
            block_bytes = serialized_entry_len(&block);
            byte_truncated = true;
            if block_bytes > remaining {
                break;
            }
        }
        bytes_returned += block_bytes;
        returned_matches += block.match_line_numbers.len();
        returned_blocks.push(block);
        if byte_truncated {
            break;
        }
    }

    let next = options.offset.saturating_add(returned_matches);
    GrepSearchResult {
        matches: returned_flat,
        blocks: returned_blocks,
        total_matches,
        returned_matches,
        bytes_returned,
        bytes_total,
        next_offset: (next < total_matches).then_some(next),
        byte_truncated,
    }
}

// Include a comma per entry so a collection of entries never exceeds the reported budget.
fn serialized_entry_len<T: Serialize>(value: &T) -> usize {
    serde_json::to_vec(value)
        .expect("grep result types are always JSON serializable")
        .len()
        .saturating_add(1)
}

fn truncate_line_to_serialized_size(item: &mut GrepMatch, max_bytes: usize) {
    let original = std::mem::take(&mut item.line);
    let mut low = 0;
    let mut high = original.len();
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        item.line = truncate_utf8(&original, mid).to_string();
        if serialized_entry_len(item) <= max_bytes {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    item.line = truncate_utf8(&original, low).to_string();
}

fn truncate_block_to_serialized_size(block: &mut GrepContextBlock, max_bytes: usize) {
    let originals: Vec<_> = block
        .lines
        .iter_mut()
        .map(|line| std::mem::take(&mut line.line))
        .collect();
    for (index, original) in originals.iter().enumerate() {
        let mut low = 0;
        let mut high = original.len();
        while low < high {
            let mid = low + (high - low).div_ceil(2);
            block.lines[index].line = truncate_utf8(original, mid).to_string();
            if serialized_entry_len(block) <= max_bytes {
                low = mid;
            } else {
                high = mid - 1;
            }
        }
        block.lines[index].line = truncate_utf8(original, low).to_string();
        if low < original.len() {
            break;
        }
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    let mut end = max_bytes.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// Grep result for a file
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GrepResult {
    pub path: String,
    pub matches: Vec<GrepMatch>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name_from_path() {
        assert_eq!(FileInfo::name_from_path("/"), "/");
        assert_eq!(FileInfo::name_from_path("/foo"), "foo");
        assert_eq!(FileInfo::name_from_path("/foo/bar"), "bar");
        assert_eq!(FileInfo::name_from_path("/foo/bar/baz.txt"), "baz.txt");
    }

    #[test]
    fn test_parent_path() {
        assert_eq!(FileInfo::parent_path("/"), None);
        assert_eq!(FileInfo::parent_path("/foo"), Some("/".to_string()));
        assert_eq!(FileInfo::parent_path("/foo/bar"), Some("/foo".to_string()));
        assert_eq!(
            FileInfo::parent_path("/foo/bar/baz"),
            Some("/foo/bar".to_string())
        );
    }

    #[test]
    fn text_heuristic_samples_only_first_eight_kibibytes() {
        for bytes in [b"".as_slice(), b"hello world", b"line1\nline2\n"] {
            assert!(SessionFile::is_text_content(bytes));
        }
        let mut bytes = vec![b'a'; 8193];
        bytes[8192] = 0;
        assert!(SessionFile::is_text_content(&bytes));
        bytes[8191] = 0;
        assert!(!SessionFile::is_text_content(&bytes));
        assert!(!SessionFile::is_text_content(b"hello\0world"));
    }

    #[test]
    fn content_encoding_preserves_exact_text_and_binary_wire_values() {
        for (input, content, encoding) in [
            (b"".as_slice(), "", "text"),
            (b"hello world".as_slice(), "hello world", "text"),
            ("éà".as_bytes(), "éà", "text"),
            (b"a\0b".as_slice(), "YQBi", "base64"),
            (b"\xff\xfe".as_slice(), "//4=", "base64"),
        ] {
            let actual = SessionFile::encode_content(input);
            assert_eq!(actual, (content.to_string(), encoding.to_string()));
            assert_eq!(
                SessionFile::decode_content(&actual.0, &actual.1).unwrap(),
                input
            );
        }
    }

    #[test]
    fn content_decoding_preserves_text_and_rejects_invalid_base64() {
        assert_eq!(
            SessionFile::decode_content("literal ! é", "text").unwrap(),
            "literal ! é".as_bytes()
        );
        assert_eq!(
            SessionFile::decode_content("aGVsbG8=", "base64").unwrap(),
            b"hello"
        );
        assert_eq!(
            SessionFile::decode_content("YQBi", "base64").unwrap(),
            b"a\0b"
        );
        for malformed in ["!", "YQ=", "===="] {
            assert!(
                SessionFile::decode_content(malformed, "base64").is_err(),
                "{malformed}"
            );
        }
    }

    #[test]
    fn merge_context_results_applies_one_match_window_without_duplicate_lines() {
        let block = |path: &str, start: usize, matches: &[usize]| GrepContextBlock {
            path: path.to_string(),
            start_line: start,
            end_line: start + 2,
            match_line_numbers: matches.to_vec(),
            lines: (start..=start + 2)
                .map(|line_number| GrepContextLine {
                    line_number,
                    line: format!("line {line_number}"),
                    is_match: matches.contains(&line_number),
                })
                .collect(),
        };
        let result = |blocks| GrepSearchResult {
            matches: Vec::new(),
            blocks,
            total_matches: 0,
            returned_matches: 0,
            bytes_returned: 0,
            bytes_total: 0,
            next_offset: None,
            byte_truncated: false,
        };
        let options = GrepOptions {
            before_context: 1,
            after_context: 1,
            offset: 1,
            limit: 2,
            ..GrepOptions::default()
        };

        let merged = merge_grep_search_results(
            vec![
                result(vec![block("/b.txt", 4, &[5])]),
                result(vec![
                    block("/a.txt", 3, &[4]),
                    block("/a.txt", 1, &[2]),
                    block("/a.txt", 3, &[4]),
                ]),
            ],
            &options,
        );

        assert_eq!(merged.total_matches, 3);
        assert_eq!(merged.returned_matches, 2);
        assert_eq!(merged.next_offset, None);
        assert_eq!(merged.blocks.len(), 2);
        assert_eq!(merged.blocks[0].match_line_numbers, vec![4]);
        assert_eq!(merged.blocks[1].match_line_numbers, vec![5]);
        assert_eq!(merged.blocks[0].path, "/a.txt");
        assert_eq!(merged.blocks[1].path, "/b.txt");
        assert_eq!(
            (merged.blocks[0].start_line, merged.blocks[0].end_line),
            (3, 5)
        );
        assert_eq!(
            (merged.blocks[1].start_line, merged.blocks[1].end_line),
            (4, 6)
        );
        assert!(!merged.byte_truncated);
        assert_eq!(
            merged.blocks[0]
                .lines
                .iter()
                .map(|line| (line.line_number, line.line.as_str(), line.is_match))
                .collect::<Vec<_>>(),
            [
                (3, "line 3", false),
                (4, "line 4", true),
                (5, "line 5", false)
            ]
        );
        assert_eq!(
            merged.blocks[1]
                .lines
                .iter()
                .map(|line| (line.line_number, line.line.as_str(), line.is_match))
                .collect::<Vec<_>>(),
            [
                (4, "line 4", false),
                (5, "line 5", true),
                (6, "line 6", false)
            ]
        );
        assert_eq!(
            merged.blocks[0]
                .lines
                .iter()
                .map(|line| line.line_number)
                .collect::<Vec<_>>(),
            vec![3, 4, 5]
        );
    }

    #[test]
    fn contextual_grep_budgets_serialized_structure() {
        let blocks = (0..40)
            .map(|index| GrepContextBlock {
                path: format!("/sparse/{index}.txt"),
                start_line: 1,
                end_line: 41,
                match_line_numbers: vec![21],
                lines: (1..=41)
                    .map(|line_number| GrepContextLine {
                        line_number,
                        line: (if line_number == 21 { "x" } else { "" }).to_string(),
                        is_match: line_number == 21,
                    })
                    .collect(),
            })
            .collect();
        let result = apply_grep_byte_budget(Vec::new(), blocks, 40, &GrepOptions::default());
        let serialized_blocks = serde_json::to_vec(&result.blocks).unwrap();

        assert!(result.byte_truncated);
        assert!(result.returned_matches > 0 && result.returned_matches < 40);
        assert!(result.bytes_total > 65_536);
        assert!(result.bytes_returned <= 65_536);
        assert!(serialized_blocks.len() <= 65_536);
    }
    #[test]
    fn flat_grep_pagination_is_global_sorted_and_handles_empty_windows() {
        let regex = regex::Regex::new("^hit").unwrap();
        let files = vec![
            ("/b".into(), "no\nhit b".into()),
            ("/a".into(), "hit a1\nnot hit\nhit a3".into()),
        ];
        for (offset, limit, expected, next) in [
            (
                0,
                2,
                vec![("/a", 1, "hit a1"), ("/a", 3, "hit a3")],
                Some(2),
            ),
            (1, 1, vec![("/a", 3, "hit a3")], Some(2)),
            (2, 2, vec![("/b", 2, "hit b")], None),
            (3, 1, vec![], None),
            (usize::MAX, 2, vec![], None),
            (0, 0, vec![], Some(0)),
        ] {
            let options = GrepOptions {
                offset,
                limit,
                ..Default::default()
            };
            let built = build_grep_search_result(files.clone(), &regex, &options);
            let unsorted = vec![
                GrepMatch {
                    path: "/b".into(),
                    line_number: 2,
                    line: "hit b".into(),
                },
                GrepMatch {
                    path: "/a".into(),
                    line_number: 3,
                    line: "hit a3".into(),
                },
                GrepMatch {
                    path: "/a".into(),
                    line_number: 1,
                    line: "hit a1".into(),
                },
            ];
            let bounded = bound_grep_matches(unsorted, &options);
            for result in [built, bounded] {
                assert_eq!(
                    result
                        .matches
                        .iter()
                        .map(|hit| (hit.path.as_str(), hit.line_number, hit.line.as_str()))
                        .collect::<Vec<_>>(),
                    expected,
                    "offset={offset} limit={limit}"
                );
                assert_eq!(result.total_matches, 3);
                assert_eq!(result.returned_matches, expected.len());
                assert_eq!(result.next_offset, next);
                assert!(!result.byte_truncated);
                assert!(result.blocks.is_empty());
            }
        }
    }

    #[test]
    fn context_builder_merges_adjacent_windows_and_marks_only_selected_matches() {
        let result = build_grep_search_result(
            vec![("/a".into(), "hit first\ncontext\nhit second\nafter".into())],
            &regex::Regex::new("^hit").unwrap(),
            &GrepOptions {
                before_context: 1,
                after_context: 1,
                ..Default::default()
            },
        );
        assert_eq!(result.total_matches, 2);
        assert_eq!(result.returned_matches, 2);
        assert_eq!(result.next_offset, None);
        assert_eq!(result.blocks.len(), 1);
        let block = &result.blocks[0];
        assert_eq!(block.path, "/a");
        assert_eq!((block.start_line, block.end_line), (1, 4));
        assert_eq!(block.match_line_numbers, [1, 3]);
        assert_eq!(
            block
                .lines
                .iter()
                .map(|line| (line.line_number, line.line.as_str(), line.is_match))
                .collect::<Vec<_>>(),
            [
                (1, "hit first", true),
                (2, "context", false),
                (3, "hit second", true),
                (4, "after", false)
            ]
        );
        let paged = build_grep_search_result(
            vec![("/a".into(), "hit first\ncontext\nhit second\nafter".into())],
            &regex::Regex::new("^hit").unwrap(),
            &GrepOptions {
                before_context: 2,
                after_context: 1,
                offset: 1,
                limit: 1,
                ..Default::default()
            },
        );
        assert_eq!(paged.blocks[0].match_line_numbers, [3]);
        assert!(!paged.blocks[0].lines[0].is_match); // skipped match is context, not a second returned match
        assert_eq!(paged.returned_matches, 1);
    }

    #[test]
    fn flat_byte_budget_preserves_utf8_and_json_escaping_at_exact_entry_boundary() {
        let input = GrepMatch {
            path: "/a".into(),
            line_number: 1,
            line: "é\"x\n😀".into(),
        };
        let expected = GrepMatch {
            line: "é".into(),
            ..input.clone()
        };
        // The API budgets serialized entries, including a comma allowance.
        let budget = br#"{"path":"/a","line_number":1,"line":""}"#.len() + "é".len() + 1;
        let result = bound_grep_matches(
            vec![
                input.clone(),
                GrepMatch {
                    path: "/b".into(),
                    line_number: 2,
                    line: "next".into(),
                },
            ],
            &GrepOptions {
                max_bytes: budget,
                ..Default::default()
            },
        );
        assert_eq!(
            serde_json::to_value(&result.matches).unwrap(),
            serde_json::json!([expected])
        );
        assert_eq!(result.bytes_returned, budget);
        assert!(result.bytes_total > budget);
        assert!(result.byte_truncated);
        assert_eq!(result.total_matches, 2);
        assert_eq!(result.returned_matches, 1);
        assert_eq!(result.next_offset, Some(1));
        for max_bytes in [0, 1] {
            let empty = bound_grep_matches(
                vec![input.clone()],
                &GrepOptions {
                    max_bytes,
                    ..Default::default()
                },
            );
            assert!(empty.matches.is_empty());
            assert_eq!(empty.bytes_returned, 0);
            assert!(empty.byte_truncated);
            assert_eq!(empty.next_offset, Some(0));
        }
    }
}
