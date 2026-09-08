// Tool Output Sanitizer
//
// Shared helpers for cleaning exec tool output before returning to LLM context.
// Each exec tool calls these directly — sanitization is the tool's responsibility.
//
// Design decisions:
// - Baked into each tool, not enforced by middleware/hooks (tool owns its output)
// - strip_ansi + collapse_cr_lines reduce noise 20-40% for build/install commands
// - middle_truncate keeps first 20% + last 80% (errors cluster at the end)
// - EXEC_OUTPUT_BUDGET = 16 KiB — industry standard is 10-30K chars
// - EVE-225 provides a separate hard limit (64 KiB) as a safety net
//
// Follow-ups:
// - EVE-222: persist_output hint drives VFS persistence via PostToolExecHook
// - EVE-223: EXEC_OUTPUT_HINT constant for system prompt additions
// - EVE-489: `auto` output mode — persistence-first defaults for exec tools

/// Legacy output budget constant (16 KiB). Kept for backward compatibility
/// with any code not yet migrated to `output_verbosity_budget()`.
/// New code should use the verbosity modes instead (default: `auto`).
pub const EXEC_OUTPUT_BUDGET: usize = 16 * 1024;

/// Output verbosity budgets (EVE-236).
/// Each exec tool accepts an `output` parameter controlling how much output
/// is returned to the LLM. The full log is always available via
/// `tool_output_persistence` (read with `read_file`).
pub const SILENT_BUDGET: usize = 200;
pub const CONCISE_BUDGET: usize = 2 * 1024;
pub const NORMAL_BUDGET: usize = 8 * 1024;
pub const VERBOSE_BUDGET: usize = 16 * 1024;

/// Compact "success summary" budget used by `auto` mode (EVE-489).
/// When an exec call succeeds and full output is persisted to `/outputs/`,
/// the inline payload only needs enough bytes to confirm completion — the
/// model can `read_file` the persisted log when it needs more detail.
///
/// Sized so that even after the `PersistOutputHook` appends its
/// `[full output saved to <display path> (NN KiB) — use read_file ...]`
/// pointer (~120 bytes), the full inline `stdout` field stays ≤ ~512 bytes.
pub const AUTO_SUCCESS_BUDGET: usize = 384;

/// Resolve output verbosity mode string to byte budget.
/// Returns `None` for "full" (no truncation).
///
/// Note: `auto` is exec-tool specific and depends on the process exit code,
/// so callers should run [`resolve_auto_mode`] first to map `auto` to a
/// concrete mode. A bare `auto` reaching this function resolves to the
/// tight `AUTO_SUCCESS_BUDGET` rather than silently falling back to
/// `CONCISE_BUDGET` — callers that forget to resolve will at worst
/// over-truncate, never silently widen.
pub fn output_verbosity_budget(mode: &str) -> Option<usize> {
    match mode {
        "silent" => Some(SILENT_BUDGET),
        "concise" => Some(CONCISE_BUDGET),
        "normal" => Some(NORMAL_BUDGET),
        "verbose" => Some(VERBOSE_BUDGET),
        "full" => None,
        "auto" | "auto_success" => Some(AUTO_SUCCESS_BUDGET),
        _ => Some(CONCISE_BUDGET), // unknown → default
    }
}

/// Resolve the exec-tool `auto` mode to a concrete verbosity mode based on
/// the process exit code (EVE-489).
///
/// - `auto` + success (`exit_code == 0`) → `auto_success` (tight summary).
///   Full output remains in `raw_output` and is persisted via
///   `ToolOutputPersistenceCapability` when the tool opts in.
/// - `auto` + failure → `normal` so the model can debug without immediately
///   reading the persisted log.
/// - Any other (explicit) mode is returned unchanged.
pub fn resolve_auto_mode(mode: &str, exit_code: i32) -> &str {
    if mode == "auto" {
        if exit_code == 0 {
            "auto_success"
        } else {
            "normal"
        }
    } else {
        mode
    }
}

/// JSON schema fragment for the `output` parameter, suitable for insertion
/// into a tool's `properties` object.
pub fn output_verbosity_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "string",
        "enum": ["auto", "silent", "concise", "normal", "verbose", "full"],
        "default": "auto",
        "description": "Output verbosity: auto (default — compact summary on success, ~8KiB on failure for diagnostics), silent (~200B), concise (~2KiB), normal (~8KiB), verbose (~16KiB), full (unlimited, capped by 64KiB hard limit). For tools with output persistence enabled, full output is saved to /outputs/{tool_call_id}.stdout and /outputs/{tool_call_id}.stderr — use read_file to retrieve."
    })
}

/// System prompt hint for exec tool capabilities (EVE-223, EVE-236, EVE-489, EVE-778).
/// Appended to each sandbox capability's `system_prompt_addition()` to guide
/// the LLM toward less verbose command usage and a single-read/contextual-search
/// policy for persisted output.
pub const EXEC_OUTPUT_HINT: &str = "\n\n**Output economy:** The `output` parameter shapes command output (default: `auto` — compact summary on success, ~8 KiB diagnostic window on failure). \
Persistence-enabled tools save full output to `/outputs/{tool_call_id}.stdout`/`.stderr`; oversized results include an `output_files` array of paths to `read_file` with offset/limit.\n\
Modes: `auto` (default), `silent` (~200B), `concise` (~2KiB), `normal` (~8KiB), `verbose` (~16KiB), `full` (unlimited).\n\
`auto` is usually sufficient — check `exit_code` first; use `verbose` or read the persisted files when more detail is needed.\n\
When the needed filter is known before running a command, apply it in the command itself (e.g. pipe through `grep`) instead of post-filtering persisted output. \
Read a persisted log at most once: ≤200 lines or ≤64 KiB fits one `read_file` with an ample `limit`; anything larger is one `grep_files` search with context. \
Never reconstruct a file through sequential or overlapping reads — stop once you have enough diagnostic evidence.";

/// System prompt hint for file reading economy (EVE-244, EVE-778).
/// Appended to the FileSystem capability's `system_prompt_addition()` to guide
/// the LLM toward efficient file reading with offset/limit pagination and a
/// single complete read for small files.
pub const READ_ECONOMY_HINT: &str = "\n\n**File reading economy:** `read_file` returns at most 2000 lines by default.\n\
- Locate the relevant region first with `grep_files`, then read that section with `read_file` using `offset` and `limit`.\n\
- Use `list_directory` to understand file structure before reading.\n\
- When a read is truncated, check `total_lines` to see how much remains and continue from `lines_shown.end` on the next call.\n\
- Read small files (≤200 lines or ≤64 KiB — most persisted `/outputs/` logs) once with an ample `limit`; search larger ones with a single contextual `grep_files` (`before_context`/`after_context`). Never rebuild a file from sequential or overlapping windows — stop once you have the evidence you need.";

/// Strip ANSI escape sequences from text.
///
/// Removes SGR sequences (`\x1b[...m`), CSI sequences (`\x1b[...X`),
/// and OSC sequences (`\x1b]...BEL/ST`). Preserves all non-escape content.
pub fn strip_ansi(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // ESC sequence — consume until terminator
            match chars.peek() {
                Some('[') => {
                    // CSI sequence: ESC [ ... (final byte 0x40..=0x7E, '@'..='~')
                    chars.next(); // consume '['
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    // OSC sequence: ESC ] ... (BEL or ESC \)
                    chars.next(); // consume ']'
                    for c in chars.by_ref() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' {
                            // ST = ESC backslash
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                Some('(') | Some(')') => {
                    // Character set designation: ESC ( X or ESC ) X
                    chars.next(); // consume '(' or ')'
                    chars.next(); // consume the charset designator
                }
                _ => {
                    // Unknown ESC sequence — skip next char
                    chars.next();
                }
            }
        } else {
            result.push(ch);
        }
    }

    result
}

/// Collapse carriage-return overwritten lines to their final content.
///
/// Lines containing `\r` (without `\n`) are "overwritten" — only the text
/// after the last `\r` on each line is kept. This handles progress bars like
/// `Downloading 45%\rDownloading 100%` → `Downloading 100%`.
pub fn collapse_cr_lines(text: &str) -> String {
    let mut result = String::with_capacity(text.len());

    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            result.push('\n');
        }
        // Remove the CRLF terminator before choosing the final overwritten
        // segment. Separators depend on input position, including empty lines.
        let line = line.strip_suffix('\r').unwrap_or(line);
        result.push_str(line.rsplit('\r').next().unwrap_or_default());
    }

    result
}

/// Middle-truncate text to fit within `max_bytes`, keeping first 20% and last 80%.
///
/// If the text is within budget, returns it unchanged. Otherwise, keeps the
/// head (command context) and tail (errors/results) with a clear marker.
/// All cuts are UTF-8 safe — never splits multi-byte characters.
pub fn middle_truncate(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }

    // Reserve space for the omission marker (generous estimate)
    let marker_budget = 80; // "[... NNNNN bytes omitted ...]" + newlines
    let content_budget = max_bytes.saturating_sub(marker_budget);
    if content_budget == 0 {
        let mut marker = format!("[... {} bytes omitted ...]", text.len());
        if marker.len() > max_bytes {
            let cutoff = utf8_floor(&marker, max_bytes);
            marker.truncate(cutoff);
        }
        return marker;
    }

    // 20% head, 80% tail
    let head_budget = content_budget / 5;
    let tail_budget = content_budget - head_budget;

    // Find UTF-8-safe cut points
    let head_end = utf8_floor(text, head_budget);
    let tail_start = utf8_ceil(text, text.len().saturating_sub(tail_budget));

    let omitted = text.len() - head_end - (text.len() - tail_start);
    let marker = format!("\n\n[... {} bytes omitted ...]\n\n", omitted);

    let mut result = String::with_capacity(head_end + marker.len() + (text.len() - tail_start));
    result.push_str(&text[..head_end]);
    result.push_str(&marker);
    result.push_str(&text[tail_start..]);
    result
}

/// Clean exec output: strip ANSI → collapse CR. No truncation.
/// Use this when you need the full cleaned output (e.g. for VFS persistence)
/// and will truncate separately.
pub fn clean_exec_output(text: &str) -> String {
    let cleaned = strip_ansi(text);
    collapse_cr_lines(&cleaned)
}

/// Default line limit for read_file (industry standard: 2000 lines).
pub const READ_FILE_DEFAULT_LIMIT: usize = 2000;

/// Hard byte cap for read_file (50 KB safety net for pathological cases like minified files).
pub const READ_FILE_HARD_BYTE_CAP: usize = 50 * 1024;

/// Apply the read_file hard byte cap to already-formatted output.
///
/// Returns true when truncation was applied.
pub fn apply_read_file_hard_cap(result: &mut String) -> bool {
    if result.len() <= READ_FILE_HARD_BYTE_CAP {
        return false;
    }

    let cut = utf8_floor(result, READ_FILE_HARD_BYTE_CAP);
    result.truncate(cut);
    true
}

#[derive(Debug)]
struct FormattedReadFileWindow {
    content: String,
    total_lines: usize,
    start_line: usize,
    end_line: usize,
    line_capped: bool,
    size_capped: bool,
}

/// Format file content with compact line numbers: `N|content`.
///
/// Applies offset/limit pagination and the hard byte cap in one pass while
/// preserving whether truncation was caused by the line window or byte budget.
/// Line numbers are 1-based in output regardless of offset.
fn format_lines_with_metadata(
    content: &str,
    offset: usize,
    limit: usize,
) -> FormattedReadFileWindow {
    let window_end = offset.saturating_add(limit);
    let mut total_lines = 0;
    let mut result = String::new();
    let mut start_line = 0;
    let mut end_line = 0;
    let mut size_capped = false;

    for (idx, line) in content.lines().enumerate() {
        total_lines = idx + 1;

        if idx < offset || idx >= window_end {
            continue;
        }

        if size_capped {
            continue;
        }

        // 1-based line numbers
        let line_num = idx + 1;
        if start_line == 0 {
            start_line = line_num;
        }

        let separator = if result.is_empty() { "" } else { "\n" };
        let formatted_line = format!("{separator}{line_num}|{line}");
        let available = READ_FILE_HARD_BYTE_CAP.saturating_sub(result.len());

        if formatted_line.len() <= available {
            result.push_str(&formatted_line);
            end_line = line_num;
            continue;
        }

        let cut = utf8_floor(&formatted_line, available);
        if cut > 0 {
            result.push_str(&formatted_line[..cut]);
            end_line = line_num;
        }
        size_capped = true;
    }

    let end = offset.saturating_add(limit).min(total_lines);
    let line_capped = end < total_lines;

    FormattedReadFileWindow {
        content: result,
        total_lines,
        start_line,
        end_line,
        line_capped,
        size_capped,
    }
}

/// Format file content with compact line numbers: `N|content`.
///
/// Applies offset/limit pagination. Returns (formatted_content, total_lines, truncated).
/// Line numbers are 1-based in output regardless of offset.
pub fn format_lines(content: &str, offset: usize, limit: usize) -> (String, usize, bool) {
    let formatted = format_lines_with_metadata(content, offset, limit);
    (
        formatted.content,
        formatted.total_lines,
        formatted.line_capped || formatted.size_capped,
    )
}

/// Parse standard read-file `offset`/`limit` arguments.
pub fn parse_read_file_window_args(
    arguments: &serde_json::Value,
) -> Result<(usize, usize), String> {
    let offset = arguments
        .get("offset")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let limit = arguments
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(READ_FILE_DEFAULT_LIMIT as u64) as usize;
    if limit == 0 {
        return Err("limit must be a positive integer".to_string());
    }
    Ok((offset, limit))
}

/// Build the standard structured response for a text read-file tool.
pub fn build_text_read_file_result(
    tool_name: &str,
    path: &str,
    content: &str,
    encoding: &str,
    offset: usize,
    limit: usize,
) -> serde_json::Value {
    let formatted = format_lines_with_metadata(content, offset, limit);
    let truncated = formatted.line_capped || formatted.size_capped;

    let mut result = serde_json::json!({
        "path": path,
        "content": formatted.content,
        "encoding": encoding,
        "total_lines": formatted.total_lines,
        "lines_shown": {
            "start": formatted.start_line,
            "end": formatted.end_line,
        },
        "truncated": truncated,
        "size_bytes": content.len(),
    });

    let truncation = if truncated {
        if formatted.size_capped {
            crate::truncation_info::TruncationInfo::without_resume(
                formatted.content.len(),
                Some(content.len()),
                crate::truncation_info::TruncationReason::SizeCap,
            )
        } else if formatted.line_capped {
            crate::truncation_info::TruncationInfo::with_resume(
                formatted.content.len(),
                Some(content.len()),
                formatted.end_line as u64,
                format!(
                    "call {tool_name} with offset={} to resume from line {}",
                    formatted.end_line,
                    formatted.end_line + 1,
                ),
                crate::truncation_info::TruncationReason::LineCap,
            )
        } else {
            crate::truncation_info::TruncationInfo::without_resume(
                formatted.content.len(),
                Some(content.len()),
                crate::truncation_info::TruncationReason::SizeCap,
            )
        }
    } else {
        crate::truncation_info::TruncationInfo::not_truncated(formatted.content.len())
    };
    truncation.attach(&mut result);

    result
}

/// Build the standard structured response for a binary read-file tool.
pub fn build_binary_read_file_result(
    path: &str,
    size_bytes: usize,
    encoding: &str,
) -> serde_json::Value {
    let mut result = serde_json::json!({
        "path": path,
        "content_type": "binary",
        "encoding": encoding,
        "size_bytes": size_bytes,
        "note": "Binary file — use a different tool or download to inspect."
    });
    crate::truncation_info::TruncationInfo {
        truncated: false,
        bytes_returned: 0,
        bytes_total: Some(size_bytes),
        next_offset: None,
        resume_hint: None,
        reason: crate::truncation_info::TruncationReason::SizeCap,
    }
    .attach(&mut result);
    result
}

/// Build the standard structured response for file bytes.
///
/// Valid UTF-8 bytes are returned through the standard line-windowed text
/// formatter. Invalid UTF-8 returns metadata only so binary payloads do not
/// leak into model context as lossy replacement text or base64.
pub fn build_bytes_read_file_result(
    tool_name: &str,
    path: &str,
    bytes: &[u8],
    offset: usize,
    limit: usize,
) -> serde_json::Value {
    match std::str::from_utf8(bytes) {
        Ok(content) => build_text_read_file_result(tool_name, path, content, "text", offset, limit),
        Err(_) => build_binary_read_file_result(path, bytes.len(), "binary"),
    }
}

/// Full sanitization pipeline: strip ANSI → collapse CR → priority-aware truncate.
pub fn sanitize_exec_output(text: &str, max_bytes: usize) -> String {
    let cleaned = clean_exec_output(text);
    priority_aware_truncate(&cleaned, max_bytes)
}

/// Truncate an exec stream according to the process outcome.
///
/// Error-priority matching is useful for failed commands, but source/search
/// output from successful commands can legitimately contain words such as
/// `error`. Treating those lines as diagnostics can hide the leading matches,
/// so successful output uses the predictable head/tail window instead.
pub fn truncate_exec_stream(text: &str, max_bytes: usize, exit_code: i32) -> String {
    if exit_code == 0 {
        middle_truncate(text, max_bytes)
    } else {
        priority_aware_truncate(text, max_bytes)
    }
}

// ============================================================================
// Priority-aware truncation (EVE-246)
// ============================================================================

/// Context lines to include around each error region.
const ERROR_CONTEXT_LINES: usize = 5;

/// Error pattern markers that indicate important diagnostic output.
const ERROR_PATTERNS: &[&str] = &[
    "error:",
    "Error:",
    "ERROR",
    "FAILED",
    "FAIL",
    "failed",
    "panic",
    "panicked at",
    "assert",
    "assertion failed",
    "Traceback (most recent call last)",
    "at Object.<anonymous>",
    "at Module._compile",
    "--- stderr ---",
];

/// Patterns that must appear at the start of a line.
const LINE_START_PATTERNS: &[&str] = &["E "];

/// A region of text identified as error-significant.
#[derive(Debug, Clone)]
struct ErrorRegion {
    /// Start line index (inclusive).
    start: usize,
    /// End line index (exclusive).
    end: usize,
}

/// Scan output lines for error-significant regions, returning merged regions
/// with ±ERROR_CONTEXT_LINES of surrounding context.
fn find_error_regions(lines: &[&str]) -> Vec<ErrorRegion> {
    let mut hit_lines: Vec<usize> = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        let is_error = ERROR_PATTERNS.iter().any(|p| line.contains(p))
            || LINE_START_PATTERNS.iter().any(|p| line.starts_with(p));
        if is_error {
            hit_lines.push(idx);
        }
    }

    if hit_lines.is_empty() {
        return Vec::new();
    }

    // Expand each hit to ±context and merge overlapping regions.
    let total = lines.len();
    let mut regions: Vec<ErrorRegion> = Vec::new();

    for &hit in &hit_lines {
        let start = hit.saturating_sub(ERROR_CONTEXT_LINES);
        let end = (hit + ERROR_CONTEXT_LINES + 1).min(total);

        if let Some(last) = regions.last_mut()
            && start <= last.end
        {
            // Merge with previous region.
            last.end = end;
            continue;
        }
        regions.push(ErrorRegion { start, end });
    }

    regions
}

/// Truncate output preserving error-significant regions.
///
/// If no error patterns are found, falls back to `middle_truncate` (zero regression).
/// When errors are found: allocates budget to error regions first, then fills
/// remaining budget with head/tail of the full output.
pub fn priority_aware_truncate(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }

    let lines: Vec<&str> = text.lines().collect();
    let regions = find_error_regions(&lines);

    if regions.is_empty() {
        return middle_truncate(text, max_bytes);
    }

    // Assemble error region text.
    let mut sections: Vec<String> = Vec::new();
    let mut error_bytes: usize = 0;

    for region in &regions {
        let region_text: String = lines[region.start..region.end].join("\n");
        error_bytes += region_text.len() + 40; // overhead for markers
        sections.push(region_text);
    }

    // For very small budgets, fall back to middle_truncate which handles this.
    let marker_overhead = 80;
    if max_bytes < marker_overhead {
        return middle_truncate(text, max_bytes);
    }
    let available_for_context = max_bytes - marker_overhead;

    if error_bytes >= available_for_context {
        // Just show error regions truncated to budget.
        let mut result = String::new();
        let mut remaining = available_for_context;

        for (i, section) in sections.iter().enumerate() {
            let marker = if i == 0 && regions[i].start > 0 {
                format!("[... {} lines above ...]\n", regions[i].start)
            } else if i > 0 {
                let gap = regions[i].start - regions[i - 1].end;
                format!("\n[... {} lines omitted ...]\n", gap)
            } else {
                String::new()
            };

            if marker.len() >= remaining {
                break;
            }
            remaining -= marker.len();
            result.push_str(&marker);

            let take = section.len().min(remaining);
            let safe_take = utf8_floor(section, take);
            result.push_str(&section[..safe_take]);
            remaining = remaining.saturating_sub(safe_take);

            if remaining == 0 {
                break;
            }
        }

        let lines_after = lines
            .len()
            .saturating_sub(regions.last().map_or(0, |r| r.end));
        if lines_after > 0 {
            let trailer = format!("\n[... {} lines below ...]", lines_after);
            if trailer.len() <= remaining {
                result.push_str(&trailer);
            }
        }

        return result;
    }

    // Error regions fit. Fill remaining budget with head/tail.
    let context_budget = available_for_context - error_bytes;
    let head_budget = context_budget / 5; // 20% head
    let tail_budget = context_budget - head_budget; // 80% tail

    let mut result = String::new();

    // Head section: accumulate lines up to head_budget without joining all lines.
    let first_region_start = regions[0].start;
    if first_region_start > 0 {
        let mut head_used = 0usize;
        let mut head_lines_kept = 0usize;

        for line in &lines[..first_region_start] {
            let needed = if head_lines_kept > 0 {
                1 + line.len()
            } else {
                line.len()
            };
            if head_used + needed > head_budget {
                break;
            }
            if head_lines_kept > 0 {
                result.push('\n');
            }
            result.push_str(line);
            head_used += needed;
            head_lines_kept += 1;
        }

        let omitted = first_region_start - head_lines_kept;
        if omitted > 0 {
            result.push_str(&format!("\n[... {} lines omitted ...]\n", omitted));
        } else {
            result.push('\n');
        }
    }

    // Error regions with gap markers between them.
    for (i, (region, section)) in regions.iter().zip(sections.iter()).enumerate() {
        if i > 0 {
            let gap = region.start - regions[i - 1].end;
            if gap > 0 {
                result.push_str(&format!("\n[... {} lines omitted ...]\n", gap));
            }
        }
        result.push_str(section);
    }

    // Tail section: accumulate lines from end up to tail_budget.
    let last_region_end = regions.last().map_or(0, |r| r.end);
    let tail_lines = &lines[last_region_end..];
    if !tail_lines.is_empty() {
        // Calculate total tail size to check if it fits entirely.
        let tail_total: usize =
            tail_lines.iter().map(|l| l.len()).sum::<usize>() + tail_lines.len().saturating_sub(1);

        if tail_total <= tail_budget {
            result.push('\n');
            for (i, line) in tail_lines.iter().enumerate() {
                if i > 0 {
                    result.push('\n');
                }
                result.push_str(line);
            }
        } else {
            // Take lines from the end until budget is exhausted.
            let mut tail_used = 0usize;
            let mut tail_start_idx = tail_lines.len();

            for i in (0..tail_lines.len()).rev() {
                let needed = tail_lines[i].len() + if i < tail_lines.len() - 1 { 1 } else { 0 };
                if tail_used + needed > tail_budget {
                    break;
                }
                tail_used += needed;
                tail_start_idx = i;
            }

            let omitted = tail_start_idx;
            result.push_str(&format!("\n[... {} lines omitted ...]\n", omitted));
            for (i, line) in tail_lines[tail_start_idx..].iter().enumerate() {
                if i > 0 {
                    result.push('\n');
                }
                result.push_str(line);
            }
        }
    }

    // Safety: ensure total doesn't exceed budget.
    if result.len() > max_bytes {
        let safe = utf8_floor(&result, max_bytes);
        result.truncate(safe);
    }

    result
}

/// Find the largest byte index ≤ `pos` that is a valid UTF-8 char boundary.
fn utf8_floor(text: &str, pos: usize) -> usize {
    let pos = pos.min(text.len());
    let mut i = pos;
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Find the smallest byte index ≥ `pos` that is a valid UTF-8 char boundary.
fn utf8_ceil(text: &str, pos: usize) -> usize {
    let pos = pos.min(text.len());
    let mut i = pos;
    while i < text.len() && !text.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    // ====================================================================
    // strip_ansi
    // ====================================================================

    #[test]
    fn ansi_sequences_are_removed_without_changing_visible_text() {
        for (input, expected) in [
            ("hello world", "hello world"),
            ("", ""),
            ("array[0] = 1", "array[0] = 1"),
            (
                "\x1b[1;31merror\x1b[0m: something failed",
                "error: something failed",
            ),
            ("\x1b[2J\x1b[Hhello", "hello"),
            ("\x1b]0;my title\x07some output", "some output"),
            ("\x1b]0;title\x1b\\output", "output"),
            (
                "\x1b[32mCompiling\x1b[0m foo v0.1.0\n\x1b[31merror\x1b[0m[E0308]: mismatched types",
                "Compiling foo v0.1.0\nerror[E0308]: mismatched types",
            ),
            ("\x1b(Bé\x1b)0€", "é€"),
            ("visible\x1b[31", "visible"),
        ] {
            assert_eq!(strip_ansi(input), expected, "input {input:?}");
        }
    }

    #[test]
    fn carriage_returns_preserve_final_updates_and_line_endings() {
        for (input, expected) in [
            ("hello\nworld", "hello\nworld"),
            (
                "Downloading 10%\rDownloading 50%\rDownloading 100%",
                "Downloading 100%",
            ),
            (
                "Building...\rBuilding... done\nTests passed\nProgress 50%\rProgress 100%",
                "Building... done\nTests passed\nProgress 100%",
            ),
            ("hello\r", "hello"),
            ("line1\r\nline2\r\n", "line1\nline2\n"),
            ("", ""),
        ] {
            assert_eq!(collapse_cr_lines(input), expected, "input {input:?}");
        }
    }

    #[test]
    fn middle_truncation_preserves_inputs_within_budget() {
        for (text, budget) in [("", 0), ("short text", 1024), ("exact", 5), ("é€", 5)] {
            assert_eq!(middle_truncate(text, budget), text);
        }
    }

    #[test]
    fn middle_truncation_retains_distinct_head_and_tail_within_budget() {
        let text = format!("HEAD_CONTENT_{}_TAIL_CONTENT", "x".repeat(1000));
        let result = middle_truncate(&text, 200);
        assert!(result.len() <= 200);
        assert!(result.starts_with("HEAD_CONTENT_"));
        assert!(result.ends_with("_TAIL_CONTENT"));
        let marker = result.find("[...").unwrap();
        let end_marker = result.find("...]").unwrap() + 4;
        assert!(result.contains("bytes omitted"));
        assert!(
            result.len() - end_marker > marker,
            "tail must receive more budget than head"
        );
        assert!(!result.contains(&"x".repeat(200)));
    }

    #[test]
    fn test_middle_truncate_utf8_safety() {
        for text in ["é".repeat(300), "€".repeat(200), "😀".repeat(150)] {
            let result = middle_truncate(&text, 100);
            assert!(result.len() <= 100);
            let (head, rest) = result.split_once("\n\n[... ").unwrap();
            let (_, tail) = rest.split_once(" ...]\n\n").unwrap();
            assert!(!head.is_empty());
            assert!(!tail.is_empty());
            assert!(text.starts_with(head));
            assert!(text.ends_with(tail));
        }
    }

    #[test]
    fn test_middle_truncate_very_small_budget() {
        let text = "a".repeat(1000);
        let marker = "[... 1000 bytes omitted ...]";
        for budget in [0, 1, 10, 25, 50, 80] {
            assert_eq!(
                middle_truncate(&text, budget),
                &marker[..budget.min(marker.len())]
            );
        }
    }

    #[test]
    fn test_sanitize_pipeline() {
        let input = format!(
            "\x1b[32mCompiling\x1b[0m foo\nProgress 50%\rProgress 100%\n{}",
            "x".repeat(20000)
        );
        let result = sanitize_exec_output(&input, 500);
        // ANSI stripped
        assert!(!result.contains("\x1b"));
        // CR collapsed
        assert!(!result.contains("Progress 50%"));
        assert!(result.contains("Progress 100%"));
        // Truncated
        assert!(result.len() <= 500);
    }

    #[test]
    fn test_sanitize_small_output_unchanged() {
        let input = "hello world";
        assert_eq!(sanitize_exec_output(input, EXEC_OUTPUT_BUDGET), input);
    }

    // ====================================================================
    // utf8 helpers
    // ====================================================================

    #[test]
    fn utf8_rounding_uses_explicit_character_boundaries() {
        for (text, position, floor, ceil) in [
            ("", 0, 0, 0),
            ("", usize::MAX, 0, 0),
            ("hello", 3, 3, 3),
            ("abc", 100, 3, 3),
            ("a€b", 0, 0, 0),
            ("a€b", 1, 1, 1),
            ("a€b", 2, 1, 4),
            ("a€b", 3, 1, 4),
            ("a€b", 4, 4, 4),
            ("a€b", 5, 5, 5),
            ("é😀", 1, 0, 2),
            ("é😀", 3, 2, 6),
            ("é😀", usize::MAX, 6, 6),
        ] {
            assert_eq!(utf8_floor(text, position), floor);
            assert_eq!(utf8_ceil(text, position), ceil);
        }
    }

    #[test]
    fn line_windows_preserve_numbering_totals_and_truncation() {
        for (text, offset, limit, expected, total, truncated) in [
            (
                "alpha\nbeta\ngamma",
                0,
                2000,
                "1|alpha\n2|beta\n3|gamma",
                3,
                false,
            ),
            ("a\nb\nc\nd\ne", 2, 2, "3|c\n4|d", 5, true),
            ("a\nb", 10, 5, "", 2, false),
            ("a\nb\nc\nd\ne", 0, 3, "1|a\n2|b\n3|c", 5, true),
            ("", 0, 2000, "", 0, false),
            ("hello", 0, 2000, "1|hello", 1, false),
            ("a\nb", usize::MAX, usize::MAX, "", 2, false),
            ("a\nb", 0, 0, "", 2, true),
            ("\n\né\n", 0, 3, "1|\n2|\n3|é", 3, false),
        ] {
            assert_eq!(
                format_lines(text, offset, limit),
                (expected.into(), total, truncated),
                "offset={offset} limit={limit}"
            );
        }
    }

    #[test]
    fn test_format_lines_hard_byte_cap() {
        let content = format!("{}\nafter", "€".repeat(18000));
        let (formatted, total, truncated) = format_lines(&content, 0, 2);
        assert_eq!(total, 2);
        assert!(truncated);
        assert_eq!(formatted, format!("1|{}", "€".repeat(17066)));
        assert_eq!(formatted.len(), 51200);
    }

    #[test]
    fn test_apply_read_file_hard_cap() {
        for count in [51199, 51200, 51201] {
            let mut text = "x".repeat(count);
            assert_eq!(apply_read_file_hard_cap(&mut text), count > 51200);
            assert_eq!(text, "x".repeat(count.min(51200)));
        }
        let mut text = format!("{}😀tail", "x".repeat(51199));
        assert!(apply_read_file_hard_cap(&mut text));
        assert_eq!(text, "x".repeat(51199));
    }

    #[test]
    fn test_build_text_read_file_result_window() {
        let result = build_text_read_file_result(
            "sandbox_read_file",
            "/workspace/a.txt",
            "a\nb\nc",
            "text",
            1,
            1,
        );
        assert_eq!(
            result,
            serde_json::json!({
                "path":"/workspace/a.txt","content":"2|b","encoding":"text","total_lines":3,
                "lines_shown":{"start":2,"end":2},"truncated":true,"size_bytes":5,
                "truncation":{"truncated":true,"bytes_returned":3,"bytes_total":5,"next_offset":2,
                    "resume_hint":"call sandbox_read_file with offset=2 to resume from line 3","reason":"line_cap"}
            })
        );
    }

    #[test]
    fn test_build_text_read_file_result_hard_cap_has_no_resume() {
        let big_line = "x".repeat(51201);
        let result =
            build_text_read_file_result("read_file", "/workspace/big.txt", &big_line, "text", 0, 1);

        assert_eq!(result["total_lines"], 1);
        assert_eq!(result["lines_shown"]["start"], 1);
        assert_eq!(result["lines_shown"]["end"], 1);
        assert_eq!(result["truncated"], true);
        assert_eq!(result["truncation"]["reason"], "size_cap");
        assert!(result["truncation"].get("next_offset").is_none());
        assert!(
            result["content"].as_str().unwrap().len() == 51200,
            "content exceeded hard byte cap"
        );
    }

    #[test]
    fn test_build_bytes_read_file_result_omits_binary_content() {
        let result = build_bytes_read_file_result(
            "sandbox_read_file",
            "/workspace/archive.zip",
            &[0xff, 0x00, 0xfe],
            0,
            READ_FILE_DEFAULT_LIMIT,
        );

        assert_eq!(result["content_type"], "binary");
        assert_eq!(result["encoding"], "binary");
        assert_eq!(result["size_bytes"], 3);
        assert_eq!(result["truncation"]["truncated"], false);
        assert_eq!(result["truncation"]["bytes_returned"], 0);
        assert_eq!(result["truncation"]["bytes_total"], 3);
        assert!(result.get("content").is_none());
    }

    #[test]
    fn read_window_arguments_apply_defaults_and_reject_zero_limit() {
        assert_eq!(
            parse_read_file_window_args(&serde_json::json!({})).unwrap(),
            (0, 2000)
        );
        assert_eq!(
            parse_read_file_window_args(&serde_json::json!({"offset":7})).unwrap(),
            (7, 2000)
        );
        assert_eq!(
            parse_read_file_window_args(&serde_json::json!({"offset":2,"limit":3})).unwrap(),
            (2, 3)
        );
        assert_eq!(
            parse_read_file_window_args(&serde_json::json!({"limit":0})).unwrap_err(),
            "limit must be a positive integer"
        );
    }

    #[test]
    fn test_priority_truncate_no_errors_falls_back_to_middle() {
        // No error patterns → same as middle_truncate.
        let text = "a\n".repeat(5000);
        let result = priority_aware_truncate(&text, 500);
        let expected = middle_truncate(&text, 500);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_priority_truncate_under_budget_unchanged() {
        let text = "short output with error: something failed";
        assert_eq!(priority_aware_truncate(text, 1024), text);
    }

    #[test]
    fn test_priority_truncate_preserves_error_in_middle() {
        // Build output where the error is in the middle, which middle_truncate would lose.
        let mut lines: Vec<String> = Vec::new();
        for i in 0..100 {
            lines.push(format!("Compiling dep-{}", i));
        }
        lines.push("error: mismatched types".to_string());
        lines.push("  --> src/main.rs:42:5".to_string());
        for i in 0..100 {
            lines.push(format!("post-error output line {}", i));
        }
        let text = lines.join("\n");
        let result = priority_aware_truncate(&text, 1000);

        assert!(
            result.contains("error: mismatched types"),
            "error line must be preserved, got: {}",
            result
        );
        assert!(
            result.contains("src/main.rs:42:5"),
            "error context must be preserved"
        );
    }

    #[test]
    fn test_priority_truncate_preserves_python_traceback() {
        let mut lines: Vec<String> = Vec::new();
        for i in 0..50 {
            lines.push(format!("installing dep {}", i));
        }
        lines.push("Traceback (most recent call last):".to_string());
        lines.push("  File \"test.py\", line 10, in <module>".to_string());
        lines.push("    raise BadValue(\"bad\")".to_string());
        lines.push("BadValue: bad".to_string());
        for i in 0..50 {
            lines.push(format!("cleanup line {}", i));
        }
        let text = lines.join("\n");
        let result = priority_aware_truncate(&text, 800);

        assert!(
            result.contains("Traceback (most recent call last):\n  File \"test.py\", line 10, in <module>\n    raise BadValue(\"bad\")\nBadValue: bad"),
            "Python traceback must be preserved"
        );
    }

    #[test]
    fn test_priority_truncate_preserves_panic() {
        let mut lines: Vec<String> = Vec::new();
        for _ in 0..80 {
            lines.push("noise line".to_string());
        }
        lines.push("thread 'main' panicked at 'index out of bounds'".to_string());
        for _ in 0..80 {
            lines.push("more noise".to_string());
        }
        let text = lines.join("\n");
        let result = priority_aware_truncate(&text, 600);

        assert!(
            result.contains("thread 'main' panicked at 'index out of bounds'"),
            "panic message must be preserved"
        );
    }

    #[test]
    fn test_priority_truncate_pytest_e_lines() {
        let mut lines: Vec<String> = Vec::new();
        for _ in 0..50 {
            lines.push("collecting tests...".to_string());
        }
        lines.push("E expected 1, got 2".to_string());
        for _ in 0..50 {
            lines.push("test summary".to_string());
        }
        let text = lines.join("\n");
        let result = priority_aware_truncate(&text, 600);

        assert!(
            result.contains("E expected 1, got 2"),
            "pytest E line must be preserved"
        );
    }

    #[test]
    fn priority_truncation_preserves_multiple_diagnostics_with_markers_and_budget() {
        let text = format!(
            "{}error: first error\n{}error: second error\n{}",
            "compiling...\n".repeat(30),
            "more compiling...\n".repeat(30),
            "finishing...\n".repeat(30)
        );
        let result = priority_aware_truncate(&text, 1000);
        assert!(result.contains("error: first error"));
        assert!(result.contains("error: second error"));
        assert!(result.contains("lines omitted"));
        assert!(result.len() <= 1000);
        assert!(result.len() < text.len());
    }

    #[test]
    fn error_regions_clamp_merge_and_preserve_distant_windows() {
        let mut lines = vec!["ok"; 60];
        for index in [0, 10, 40, 59] {
            lines[index] = "error: diagnostic";
        }
        lines[25] = "NOTE E ordinary text";
        let regions = find_error_regions(&lines);
        assert_eq!(
            regions
                .iter()
                .map(|region| (region.start, region.end))
                .collect::<Vec<_>>(),
            [(0, 16), (35, 46), (54, 60)]
        );
    }

    #[test]
    fn output_modes_select_effective_budgets_and_bound_actual_streams() {
        let text = format!("HEAD_{}_TAIL", "x".repeat(20000));
        for (mode, exit_code, effective, budget) in [
            ("auto", 0, "auto_success", Some(384)),
            ("auto", 1, "normal", Some(8192)),
            ("auto", -1, "normal", Some(8192)),
            ("auto", 137, "normal", Some(8192)),
            ("silent", 0, "silent", Some(200)),
            ("concise", 1, "concise", Some(2048)),
            ("normal", 0, "normal", Some(8192)),
            ("verbose", 1, "verbose", Some(16384)),
            ("full", 0, "full", None),
            ("unknown", 1, "unknown", Some(2048)),
        ] {
            let resolved = resolve_auto_mode(mode, exit_code);
            assert_eq!(resolved, effective);
            let actual_budget = output_verbosity_budget(resolved);
            assert_eq!(actual_budget, budget);
            let output = actual_budget.map_or_else(
                || text.clone(),
                |cap| truncate_exec_stream(&text, cap, exit_code),
            );
            if let Some(cap) = budget {
                assert!(output.len() <= cap);
                assert!(output.starts_with("HEAD_"));
                assert!(output.ends_with("_TAIL"));
            } else {
                assert_eq!(output, text);
            }
        }
        assert_eq!(output_verbosity_budget("auto"), Some(384));
        assert_eq!(output_verbosity_budget("auto_success"), Some(384));
    }

    #[test]
    fn collapse_cr_preserves_leading_and_blank_lines() {
        for input in ["\nfirst", "\n\nfirst\n", "\n", "\n\n"] {
            assert_eq!(collapse_cr_lines(input), input);
        }
    }

    #[test]
    fn collapse_cr_keeps_only_final_progress_update_before_crlf() {
        assert_eq!(
            collapse_cr_lines("10%\r50%\r100%\r\nnext\r\n"),
            "100%\nnext\n"
        );
    }
    #[test]
    fn successful_stream_keeps_head_tail_while_failure_prioritizes_diagnostics() {
        let text = format!(
            "HEAD_START\n{}error: buried diagnostic\n{}TAIL_END",
            "ordinary build line\n".repeat(100),
            "ordinary cleanup line\n".repeat(100)
        );
        let success = truncate_exec_stream(&text, 1000, 0);
        let failure = truncate_exec_stream(&text, 1000, 1);
        assert!(success.starts_with("HEAD_START"));
        assert!(success.ends_with("TAIL_END"));
        assert!(!success.contains("buried diagnostic"));
        assert!(failure.contains("error: buried diagnostic"));
        assert!(success.len() <= 1000);
        assert!(failure.len() <= 1000);
    }
}
