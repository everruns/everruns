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

/// Legacy output budget constant (16 KiB). Kept for backward compatibility
/// with any code not yet migrated to `output_verbosity_budget()`.
/// New code should use the verbosity modes instead (default: `concise` = 2 KiB).
pub const EXEC_OUTPUT_BUDGET: usize = 16 * 1024;

/// Output verbosity budgets (EVE-236).
/// Each exec tool accepts an `output` parameter controlling how much output
/// is returned to the LLM. The full log is always available via
/// `tool_output_persistence` (read with `read_file`).
pub const SILENT_BUDGET: usize = 200;
pub const CONCISE_BUDGET: usize = 2 * 1024;
pub const NORMAL_BUDGET: usize = 8 * 1024;
pub const VERBOSE_BUDGET: usize = 16 * 1024;

/// Resolve output verbosity mode string to byte budget.
/// Returns `None` for "full" (no truncation).
pub fn output_verbosity_budget(mode: &str) -> Option<usize> {
    match mode {
        "silent" => Some(SILENT_BUDGET),
        "concise" => Some(CONCISE_BUDGET),
        "normal" => Some(NORMAL_BUDGET),
        "verbose" => Some(VERBOSE_BUDGET),
        "full" => None,
        _ => Some(CONCISE_BUDGET), // unknown → default
    }
}

/// JSON schema fragment for the `output` parameter, suitable for insertion
/// into a tool's `properties` object.
pub fn output_verbosity_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "string",
        "enum": ["silent", "concise", "normal", "verbose", "full"],
        "default": "concise",
        "description": "Output verbosity: silent (~200B, truncated to exit code + minimal output), concise (~2KiB, default), normal (~8KiB), verbose (~16KiB), full (unlimited, capped by 64KiB hard limit). Full logs always available via read_file on /.outputs/."
    })
}

/// System prompt hint for exec tool capabilities (EVE-223, EVE-236).
/// Appended to each sandbox capability's `system_prompt_addition()` to guide
/// the LLM toward less verbose command usage.
pub const EXEC_OUTPUT_HINT: &str = "\n\n**Output economy:** Command output is truncated based on the `output` parameter (default: `concise` ~2 KiB). \
Use `verbose` or `full` when debugging failures. Full logs are always persisted to `/.outputs/` and readable with `read_file`.\n\
Available modes: `silent` (~200B), `concise` (~2KiB), `normal` (~8KiB), `verbose` (~16KiB), `full` (unlimited).\n\
For build/install commands, the default `concise` is usually sufficient — check exit code first.\n\
If you need more detail, re-run with `output: \"verbose\"` or read the full log from `/.outputs/`.";

/// System prompt hint for file reading economy (EVE-244).
/// Appended to the FileSystem capability's `system_prompt_addition()` to guide
/// the LLM toward efficient file reading with offset/limit pagination.
pub const READ_ECONOMY_HINT: &str = "\n\n**File reading economy:** `read_file` returns at most 2000 lines by default.\n\
Use `offset` and `limit` to read specific sections of large files.\n\
- Use `grep_files` to find relevant lines before reading\n\
- Use `list_directory` for file structure\n\
- Don't read entire large files when you only need a specific section\n\
- When a read is truncated, check `total_lines` to see how much remains\n\
- For files you've already read, use offset to continue from where you left off";

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

    for line in text.split('\n') {
        if !result.is_empty() {
            result.push('\n');
        }

        // Find the last \r in this line — everything before it is overwritten,
        // except when the \r is a trailing CR from a CRLF sequence. In that
        // case, keep the content before the \r instead of dropping it.
        if let Some(pos) = line.rfind('\r') {
            if pos + 1 == line.len() {
                // Trailing \r (likely from CRLF): keep content before it.
                result.push_str(&line[..pos]);
            } else {
                // In-line \r used for overwriting: keep content after it.
                result.push_str(&line[pos + 1..]);
            }
        } else {
            result.push_str(line);
        }
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

/// Format file content with compact line numbers: `N|content`.
///
/// Applies offset/limit pagination. Returns (formatted_content, total_lines, truncated).
/// Line numbers are 1-based in output regardless of offset.
/// Single-pass: counts total lines while only formatting the requested window.
pub fn format_lines(content: &str, offset: usize, limit: usize) -> (String, usize, bool) {
    let window_end = offset.saturating_add(limit);
    let mut total_lines = 0;
    let mut result = String::new();

    for (idx, line) in content.lines().enumerate() {
        total_lines = idx + 1;

        if idx < offset || idx >= window_end {
            continue;
        }

        if !result.is_empty() {
            result.push('\n');
        }

        // 1-based line numbers
        let line_num = idx + 1;
        result.push_str(&line_num.to_string());
        result.push('|');
        result.push_str(line);
    }

    let end = offset.saturating_add(limit).min(total_lines);
    let truncated = end < total_lines;

    // Apply hard byte cap
    if result.len() > READ_FILE_HARD_BYTE_CAP {
        let cut = utf8_floor(&result, READ_FILE_HARD_BYTE_CAP);
        result.truncate(cut);
        return (result, total_lines, true);
    }

    (result, total_lines, truncated)
}

/// Full sanitization pipeline: strip ANSI → collapse CR → priority-aware truncate.
pub fn sanitize_exec_output(text: &str, max_bytes: usize) -> String {
    let cleaned = clean_exec_output(text);
    priority_aware_truncate(&cleaned, max_bytes)
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
    fn test_strip_ansi_no_escapes() {
        assert_eq!(strip_ansi("hello world"), "hello world");
    }

    #[test]
    fn test_strip_ansi_sgr_color_codes() {
        // Bold red "error" then reset
        assert_eq!(
            strip_ansi("\x1b[1;31merror\x1b[0m: something failed"),
            "error: something failed"
        );
    }

    #[test]
    fn test_strip_ansi_cursor_movement() {
        // CSI H (cursor position) and CSI J (erase display)
        assert_eq!(strip_ansi("\x1b[2J\x1b[Hhello"), "hello");
    }

    #[test]
    fn test_strip_ansi_osc_title() {
        // OSC 0 (set window title) terminated by BEL
        assert_eq!(strip_ansi("\x1b]0;my title\x07some output"), "some output");
    }

    #[test]
    fn test_strip_ansi_osc_terminated_by_st() {
        // OSC terminated by ESC backslash (ST)
        assert_eq!(strip_ansi("\x1b]0;title\x1b\\output"), "output");
    }

    #[test]
    fn test_strip_ansi_preserves_normal_brackets() {
        assert_eq!(strip_ansi("array[0] = 1"), "array[0] = 1");
    }

    #[test]
    fn test_strip_ansi_mixed_content() {
        let input =
            "\x1b[32mCompiling\x1b[0m foo v0.1.0\n\x1b[31merror\x1b[0m[E0308]: mismatched types";
        assert_eq!(
            strip_ansi(input),
            "Compiling foo v0.1.0\nerror[E0308]: mismatched types"
        );
    }

    #[test]
    fn test_strip_ansi_empty() {
        assert_eq!(strip_ansi(""), "");
    }

    // ====================================================================
    // collapse_cr_lines
    // ====================================================================

    #[test]
    fn test_collapse_cr_no_cr() {
        assert_eq!(collapse_cr_lines("hello\nworld"), "hello\nworld");
    }

    #[test]
    fn test_collapse_cr_progress_bar() {
        let input = "Downloading 10%\rDownloading 50%\rDownloading 100%";
        assert_eq!(collapse_cr_lines(input), "Downloading 100%");
    }

    #[test]
    fn test_collapse_cr_mixed_lines() {
        let input = "Building...\rBuilding... done\nTests passed\nProgress 50%\rProgress 100%";
        assert_eq!(
            collapse_cr_lines(input),
            "Building... done\nTests passed\nProgress 100%"
        );
    }

    #[test]
    fn test_collapse_cr_trailing_cr() {
        // Trailing CR (from CRLF): keep content before it
        assert_eq!(collapse_cr_lines("hello\r"), "hello");
    }

    #[test]
    fn test_collapse_cr_crlf_preserved() {
        // CRLF line endings — content should be preserved
        assert_eq!(collapse_cr_lines("line1\r\nline2\r\n"), "line1\nline2\n");
    }

    #[test]
    fn test_collapse_cr_empty() {
        assert_eq!(collapse_cr_lines(""), "");
    }

    // ====================================================================
    // middle_truncate
    // ====================================================================

    #[test]
    fn test_middle_truncate_under_budget() {
        let text = "short text";
        assert_eq!(middle_truncate(text, 1024), text);
    }

    #[test]
    fn test_middle_truncate_exact_budget() {
        let text = "a".repeat(100);
        assert_eq!(middle_truncate(&text, 100), text);
    }

    #[test]
    fn test_middle_truncate_over_budget() {
        let text = "a".repeat(1000);
        let result = middle_truncate(&text, 200);
        assert!(result.len() <= 200);
        assert!(result.contains("[..."));
        assert!(result.contains("bytes omitted"));
        // Tail should be longer than head (80/20 split)
        let marker_pos = result.find("[...").unwrap();
        let after_marker = result.find("...]").unwrap() + 4;
        let head_len = marker_pos;
        let tail_len = result.len() - after_marker;
        assert!(
            tail_len > head_len,
            "tail ({}) should be > head ({})",
            tail_len,
            head_len
        );
    }

    #[test]
    fn test_middle_truncate_utf8_safety() {
        // Use 3-byte chars (€) to test that we don't split mid-character
        let text = "€".repeat(200); // 600 bytes
        let result = middle_truncate(&text, 100);
        // Must be valid UTF-8 (would panic on String construction if not)
        assert!(result.len() <= 100 + 80); // content + marker overhead
        assert!(result.contains("[..."));
    }

    #[test]
    fn test_middle_truncate_very_small_budget() {
        let text = "a".repeat(1000);
        let result = middle_truncate(&text, 50);
        assert!(result.contains("bytes omitted"));
    }

    #[test]
    fn test_middle_truncate_preserves_head_and_tail() {
        let text = format!(
            "{}{}{}",
            "HEAD_CONTENT_",
            "x".repeat(10000),
            "_TAIL_CONTENT"
        );
        let result = middle_truncate(&text, 500);
        assert!(result.starts_with("HEAD_CONTENT_"));
        assert!(result.ends_with("_TAIL_CONTENT"));
    }

    // ====================================================================
    // sanitize_exec_output (full pipeline)
    // ====================================================================

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
        assert!(result.len() <= 500 + 80); // content + marker
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
    fn test_utf8_floor_ascii() {
        assert_eq!(utf8_floor("hello", 3), 3);
    }

    #[test]
    fn test_utf8_floor_multibyte() {
        let text = "a€b"; // a(1) €(3) b(1) = 5 bytes
        assert_eq!(utf8_floor(text, 2), 1); // Can't split €, go back to 1
        assert_eq!(utf8_floor(text, 4), 4); // After €
    }

    #[test]
    fn test_utf8_ceil_multibyte() {
        let text = "a€b"; // a(1) €(3) b(1)
        assert_eq!(utf8_ceil(text, 2), 4); // Can't split €, advance to after it
    }

    #[test]
    fn test_utf8_floor_beyond_len() {
        assert_eq!(utf8_floor("abc", 100), 3);
    }

    #[test]
    fn test_utf8_ceil_beyond_len() {
        assert_eq!(utf8_ceil("abc", 100), 3);
    }

    // ====================================================================
    // format_lines
    // ====================================================================

    #[test]
    fn test_format_lines_basic() {
        let (content, total, truncated) = format_lines("alpha\nbeta\ngamma", 0, 2000);
        assert_eq!(content, "1|alpha\n2|beta\n3|gamma");
        assert_eq!(total, 3);
        assert!(!truncated);
    }

    #[test]
    fn test_format_lines_with_offset() {
        let (content, total, truncated) = format_lines("a\nb\nc\nd\ne", 2, 2);
        assert_eq!(content, "3|c\n4|d");
        assert_eq!(total, 5);
        assert!(truncated);
    }

    #[test]
    fn test_format_lines_offset_beyond_end() {
        let (content, total, truncated) = format_lines("a\nb", 10, 5);
        assert_eq!(content, "");
        assert_eq!(total, 2);
        assert!(!truncated);
    }

    #[test]
    fn test_format_lines_limit_clips() {
        let (content, total, truncated) = format_lines("a\nb\nc\nd\ne", 0, 3);
        assert_eq!(content, "1|a\n2|b\n3|c");
        assert_eq!(total, 5);
        assert!(truncated);
    }

    #[test]
    fn test_format_lines_empty_content() {
        let (content, total, truncated) = format_lines("", 0, 2000);
        assert_eq!(content, "");
        assert_eq!(total, 0);
        assert!(!truncated);
    }

    #[test]
    fn test_format_lines_hard_byte_cap() {
        // Create content that exceeds 50 KB when formatted
        let big_line = "x".repeat(1000);
        let content = (0..100)
            .map(|_| big_line.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let (formatted, total, truncated) = format_lines(&content, 0, 100);
        assert_eq!(total, 100);
        assert!(truncated);
        assert!(formatted.len() <= READ_FILE_HARD_BYTE_CAP);
        // Must be valid UTF-8 (would panic on access if not)
        assert!(formatted.is_char_boundary(formatted.len()));
    }

    #[test]
    fn test_format_lines_single_line() {
        let (content, total, truncated) = format_lines("hello", 0, 2000);
        assert_eq!(content, "1|hello");
        assert_eq!(total, 1);
        assert!(!truncated);
    }

    // ====================================================================
    // priority_aware_truncate (EVE-246)
    // ====================================================================

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
        lines.push("    raise ValueError(\"bad\")".to_string());
        lines.push("ValueError: bad".to_string());
        for i in 0..50 {
            lines.push(format!("cleanup line {}", i));
        }
        let text = lines.join("\n");
        let result = priority_aware_truncate(&text, 800);

        assert!(
            result.contains("Traceback (most recent call last)"),
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
            result.contains("panicked at"),
            "panic message must be preserved"
        );
    }

    #[test]
    fn test_priority_truncate_pytest_e_lines() {
        let mut lines: Vec<String> = Vec::new();
        for _ in 0..50 {
            lines.push("collecting tests...".to_string());
        }
        lines.push("E AssertionError: expected 1, got 2".to_string());
        for _ in 0..50 {
            lines.push("test summary".to_string());
        }
        let text = lines.join("\n");
        let result = priority_aware_truncate(&text, 600);

        assert!(
            result.contains("E AssertionError"),
            "pytest E line must be preserved"
        );
    }

    #[test]
    fn test_priority_truncate_multiple_error_regions() {
        let mut lines: Vec<String> = Vec::new();
        for _ in 0..30 {
            lines.push("compiling...".to_string());
        }
        lines.push("error: first error".to_string());
        for _ in 0..30 {
            lines.push("more compiling...".to_string());
        }
        lines.push("error: second error".to_string());
        for _ in 0..30 {
            lines.push("finishing...".to_string());
        }
        let text = lines.join("\n");
        let result = priority_aware_truncate(&text, 1000);

        assert!(result.contains("error: first error"));
        assert!(result.contains("error: second error"));
    }

    #[test]
    fn test_priority_truncate_omission_markers() {
        let mut lines: Vec<String> = Vec::new();
        for _ in 0..100 {
            lines.push("x".repeat(20));
        }
        lines.push("FAILED test case".to_string());
        for _ in 0..100 {
            lines.push("y".repeat(20));
        }
        let text = lines.join("\n");
        let result = priority_aware_truncate(&text, 800);

        assert!(
            result.contains("lines omitted")
                || result.contains("lines above")
                || result.contains("lines below"),
            "must include omission markers"
        );
    }

    #[test]
    fn test_priority_truncate_respects_budget() {
        let mut lines: Vec<String> = Vec::new();
        for i in 0..500 {
            lines.push(format!("line {} {}", i, "x".repeat(50)));
        }
        lines.push("error: something broke".to_string());
        for i in 0..500 {
            lines.push(format!("line {} {}", i + 500, "y".repeat(50)));
        }
        let text = lines.join("\n");
        let budget = 2000;
        let result = priority_aware_truncate(&text, budget);

        assert!(
            result.len() <= budget,
            "result ({} bytes) must not exceed budget ({})",
            result.len(),
            budget
        );
    }

    #[test]
    fn test_find_error_regions_empty() {
        let lines: Vec<&str> = vec!["hello", "world", "ok"];
        assert!(find_error_regions(&lines).is_empty());
    }

    #[test]
    fn test_find_error_regions_merges_nearby() {
        let mut lines: Vec<&str> = vec!["ok"; 5];
        lines.push("error: first");
        lines.extend(std::iter::repeat_n("ok", 3));
        lines.push("error: second"); // within context window of first
        lines.extend(std::iter::repeat_n("ok", 20));

        let regions = find_error_regions(&lines);
        // Should merge into one region since they're within 2*ERROR_CONTEXT_LINES+1 of each other
        assert_eq!(regions.len(), 1, "nearby errors should merge");
    }
}
