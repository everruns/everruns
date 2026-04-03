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

/// Default output budget for exec tools (16 KiB).
pub const EXEC_OUTPUT_BUDGET: usize = 16 * 1024;

/// System prompt hint for exec tool capabilities (EVE-223).
/// Appended to each sandbox capability's `system_prompt_addition()` to guide
/// the LLM toward less verbose command usage.
pub const EXEC_OUTPUT_HINT: &str = "\n\n**Output economy:** Command output is truncated to ~16 KiB (keeping first 20% + last 80%). \
For build/install commands, prefer quiet flags or pipe through tail:\n\
- `cargo build -q`, `cargo test -- --quiet`\n\
- `npm install --silent`, `npm test 2>&1 | tail -50`\n\
- `pip install -q`, `make -s`\n\
- `apt-get install -qq -y`\n\
Save verbose output to a file and inspect selectively: `cmd > /tmp/out.log 2>&1 && tail -100 /tmp/out.log`";

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

/// Full sanitization pipeline: strip ANSI → collapse CR → middle-truncate.
pub fn sanitize_exec_output(text: &str, max_bytes: usize) -> String {
    let cleaned = clean_exec_output(text);
    middle_truncate(&cleaned, max_bytes)
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
}
