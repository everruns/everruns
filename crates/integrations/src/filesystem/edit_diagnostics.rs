//! Recovery hints only. Never feed a suggested match back into the edit planner.

use super::LineEnding;
use similar::TextDiff;
use std::fmt::Write;

const MAX_SCAN_BYTES: usize = 64 * 1024;
const MAX_SCAN_LINES: usize = 1_000;
const MAX_LINE_BYTES: usize = 256;
const MAX_HINTS: usize = 3;

pub(super) fn missing_match(
    body: &str,
    old_text: &str,
    new_text: &str,
    line_ending: LineEnding,
    edit_number: usize,
) -> String {
    let mut error = format!(
        "Edit {edit_number}: Could not find an exact match for old_text: {}.",
        quoted_excerpt(old_text, 80)
    );

    // This is evidence about this hunk in the original read snapshot, never about
    // a partially edited buffer or completion of the whole batch. Short literals,
    // deletions, substrings and duplicate replacements are too weak as evidence.
    if unique_complete_lines(body, new_text) {
        error.push_str(
            " Its new_text already appears once as complete lines in the current file; \
             this edit may already be applied. Verify the location before retrying.",
        );
    }

    let separator = match line_ending {
        LineEnding::Cr => '\r',
        LineEnding::Lf | LineEnding::Crlf => '\n',
    };
    let hints = closest_lines(body, old_text, separator);
    if !hints.is_empty() {
        error.push_str(
            "\nSimilar lines in the current file (compared with the first nonblank old_text \
             line; hints only, search limited to the first 64 KiB / 1000 lines):",
        );
        for (_, number, line) in hints {
            let _ = write!(error, "\n{number}: {}", quoted_excerpt(line, 160));
        }
    }
    error.push_str(
        "\nNo edits were applied. Read the file again and use exact text before retrying.",
    );
    error
}

fn unique_complete_lines(body: &str, new_text: &str) -> bool {
    if new_text
        .chars()
        .filter(|c| !c.is_whitespace())
        .take(16)
        .count()
        < 16
    {
        return false;
    }
    let Some(start) = body.find(new_text) else {
        return false;
    };
    let end = start + new_text.len();
    // Include overlapping occurrences: match_indices would miss repeated blocks
    // sharing a line, making the uniqueness hint misleading.
    let next = start + new_text.chars().next().map_or(0, char::len_utf8);
    !body[next..].contains(new_text)
        && (start == 0 || body[..start].ends_with(['\r', '\n']))
        && (end == body.len()
            || new_text.ends_with(['\r', '\n'])
            || body[end..].starts_with(['\r', '\n']))
}

fn closest_lines<'a>(body: &'a str, old_text: &str, separator: char) -> Vec<(f32, usize, &'a str)> {
    // THREAT[TM-FS-024]: bound the search, comparisons, and returned file excerpts.
    // Do not score truncated lines: an identical prefix can hide a different target.
    if old_text.len() > MAX_SCAN_BYTES {
        return Vec::new();
    }
    let Some(target) = old_text
        .split(separator)
        .find(|line| !line.trim().is_empty())
    else {
        return Vec::new();
    };
    if target.len() > MAX_LINE_BYTES {
        return Vec::new();
    }
    let target = target.trim();
    if target.chars().count() < 4 {
        return Vec::new();
    }
    let scan_end = body.floor_char_boundary(body.len().min(MAX_SCAN_BYTES));
    let mut hints = Vec::with_capacity(MAX_HINTS + 1);
    let mut consumed = 0;
    for (index, chunk) in body[..scan_end]
        .split_inclusive(separator)
        .take(MAX_SCAN_LINES)
        .enumerate()
    {
        consumed += chunk.len();
        if consumed == scan_end && scan_end < body.len() && !chunk.ends_with(separator) {
            break;
        }
        let line = chunk.trim_end_matches(['\r', '\n']);
        if line.len() > MAX_LINE_BYTES || line.trim().is_empty() {
            continue;
        }
        let score = TextDiff::from_chars(target, line.trim()).ratio();
        if score < 0.6 {
            continue;
        }
        hints.push((score, index + 1, line));
        hints.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        hints.truncate(MAX_HINTS);
    }
    hints
}

fn quoted_excerpt(text: &str, limit: usize) -> String {
    // Escape control characters so file content cannot forge diagnostic lines.
    let mut chars = text.chars();
    let mut excerpt: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        excerpt.push('…');
    }
    serde_json::Value::String(excerpt).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_useful_lines_with_stable_line_numbers() {
        let body = "unrelated\n  let enabled = false;\nlet disabled = false;\nlet enabled=false;\n";
        let hints = closest_lines(body, "\nlet enabled=false;\nmore context", '\n');
        assert_eq!(
            hints.iter().map(|hint| hint.1).collect::<Vec<_>>(),
            [4, 2, 3]
        );
        assert!(closest_lines("unrelated", "let enabled=false;", '\n').is_empty());
    }

    #[test]
    fn bounds_search_and_ignores_incomplete_or_oversized_lines() {
        let body = format!("{}let enabled = false;\n", "\n".repeat(MAX_SCAN_LINES));
        assert!(closest_lines(&body, "let enabled=false;", '\n').is_empty());
        let body = format!("{}\nlet enabled = false;", "x".repeat(MAX_SCAN_BYTES));
        assert!(closest_lines(&body, "let enabled=false;", '\n').is_empty());
        let target = "a".repeat(MAX_LINE_BYTES);
        let body = format!("{}\n{}DIFFERENT", "x".repeat(MAX_SCAN_BYTES - 200), target);
        assert!(closest_lines(&body, &target, '\n').is_empty());
        assert!(closest_lines(&format!("{target}extra"), &target, '\n').is_empty());
        assert!(closest_lines(&target, &format!("{target}extra"), '\n').is_empty());
        // The byte window can end inside a multi-byte character.
        let body = format!("{}🙂\n", "x".repeat(MAX_SCAN_BYTES - 1));
        assert!(closest_lines(&body, "xxxx", '\n').is_empty());
    }

    #[test]
    fn bounds_and_quotes_diagnostic_output() {
        let old = format!("{}x", "\u{0001}".repeat(200));
        let line = format!("{}y\n", "\u{0001}".repeat(200));
        let error = missing_match(&line.repeat(100), &old, "", LineEnding::Lf, 1);
        assert!(error.len() < 4096, "{} bytes", error.len());
        assert_eq!(
            error.lines().filter(|line| line.contains(": \"")).count(),
            4
        );
        assert!(!error.contains('\u{0001}'));
        assert!(error.contains("\\u0001"));
        assert!(error.contains('…'));
    }

    #[test]
    fn replacement_evidence_rejects_weak_or_ambiguous_matches() {
        for (body, replacement) in [
            ("anything", ""),
            ("\n    \n", "    "),
            ("true\n", "true"),
            (
                "let enabled = true;\nlet enabled = true;\n",
                "let enabled = true;",
            ),
            ("abcdefgh\nabcdefgh\nabcdefgh\n", "abcdefgh\nabcdefgh\n"),
            ("prefix let enabled = true;\n", "let enabled = true;"),
            ("let enabled = true; suffix\n", "let enabled = true;"),
        ] {
            assert!(!unique_complete_lines(body, replacement), "{body:?}");
            assert!(
                !missing_match(body, "missing", replacement, LineEnding::Lf, 1)
                    .contains("may already be applied")
            );
        }
        assert!(unique_complete_lines(
            "let enabled = true;",
            "let enabled = true;"
        ));
        assert!(unique_complete_lines(
            "header\nlet enabled = true;\nfooter",
            "let enabled = true;\n"
        ));
    }
}
