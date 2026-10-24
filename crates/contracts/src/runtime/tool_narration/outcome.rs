//! Narration that says what came out, not only what was attempted.
//!
//! Shell and file-search tools are the bulk of a coding turn's steps. Their
//! finished line reads the tool result: "Ran `cargo test`, 3 of 12 tests
//! failed", "Found 8 matches for parse in 3 files". Only counts the tool
//! printed are reported; when nothing recognizable is there the line stays
//! the plain "Ran `cargo test`".

use std::collections::BTreeSet;
use std::sync::LazyLock;

use serde_json::Value;

use super::{ToolNarrationContext, ToolNarrationPhase, arg_str, is_uk, phrase3, pick, truncate};

/// Shell/exec command narration ("Ran `cargo test`"). `fallback` is used when
/// no command argument is present (typically the tool display name). The
/// completed phase adds the test tally or a non-zero exit code from the result.
pub fn narrate_shell_exec(
    arguments: &Value,
    fallback: &str,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
    ctx: ToolNarrationContext<'_>,
) -> String {
    let command = arg_str(arguments, &["commands", "command"])
        .map(|value| format!("`{}`", truncate(value, 48)))
        .unwrap_or_else(|| fallback.to_string());
    let verbs = pick(
        locale,
        ("Running", "Ran", "Failed to run"),
        ("Запускаю", "Запустив", "Не вдалося запустити"),
    );
    let line = phrase3(verbs, Some(command), phase);
    if phase != ToolNarrationPhase::Completed {
        return line;
    }
    match ctx.result.and_then(shell_outcome) {
        Some(outcome) => format!("{line}, {}", outcome.phrase(is_uk(locale))),
        None => line,
    }
}

/// `grep_files` narration. The completed phase says how many matches it found.
pub fn narrate_grep_files(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
    ctx: ToolNarrationContext<'_>,
) -> String {
    let pattern = arg_str(arguments, &["pattern"]).map(|pattern| truncate(pattern, 36));
    let uk = is_uk(locale);
    if phase == ToolNarrationPhase::Completed
        && let Some(found) = ctx.result.and_then(grep_outcome)
    {
        return found.phrase(pattern.as_deref(), uk);
    }
    if uk {
        match pattern {
            Some(pattern) => phrase3(
                ("Шукаю", "Знайшов", "Не вдалося знайти"),
                Some(format!("`{pattern}` у файлах")),
                phase,
            ),
            None => phrase3(
                (
                    "Шукаю у файлах",
                    "Завершив пошук у файлах",
                    "Не вдалося виконати пошук у файлах",
                ),
                None,
                phase,
            ),
        }
    } else {
        let target = pattern
            .map(|pattern| format!("files for {pattern}"))
            .unwrap_or_else(|| "files".to_string());
        phrase3(
            ("Searching", "Searched", "Failed to search"),
            Some(target),
            phase,
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ShellOutcome {
    Tests { passed: u64, failed: u64 },
    Exit(i64),
}

impl ShellOutcome {
    fn phrase(&self, uk: bool) -> String {
        match *self {
            ShellOutcome::Tests { passed, failed } => {
                let total = passed + failed;
                match (failed, uk) {
                    (0, false) if total == 1 => "1 test passed".to_string(),
                    (0, false) => format!("{total} tests passed"),
                    (_, false) => format!("{failed} of {total} tests failed"),
                    (0, true) => format!("тестів пройдено: {total}"),
                    (_, true) => format!("не пройшли тести: {failed} з {total}"),
                }
            }
            ShellOutcome::Exit(code) if uk => format!("код виходу {code}"),
            ShellOutcome::Exit(code) => format!("exit code {code}"),
        }
    }
}

/// The text a shell result printed, whatever shape the backend returns.
fn shell_text(result: &Value) -> String {
    match result {
        Value::String(text) => text.clone(),
        Value::Object(fields) => ["stdout", "output", "stderr"]
            .iter()
            .filter_map(|key| fields.get(*key).and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn shell_outcome(result: &Value) -> Option<ShellOutcome> {
    if let Some(tally) = test_tally(&shell_text(result)) {
        return Some(tally);
    }
    result
        .get("exit_code")
        .and_then(Value::as_i64)
        .filter(|code| *code != 0)
        .map(ShellOutcome::Exit)
}

#[expect(
    clippy::unwrap_used,
    reason = "constant regexes are exercised by unit tests"
)]
static ANSI: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\x1b\[[0-9;]*[A-Za-z]").unwrap());
#[expect(
    clippy::unwrap_used,
    reason = "constant regexes are exercised by unit tests"
)]
static CARGO: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"test result: (?:ok|FAILED)\. (?P<passed>\d+) passed; (?P<failed>\d+) failed",
    )
    .unwrap()
});
#[expect(
    clippy::unwrap_used,
    reason = "constant regexes are exercised by unit tests"
)]
static JEST: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?m)^\s*Tests:?\s+(?P<body>.*\d+ (?:passed|failed).*)$").unwrap()
});
#[expect(
    clippy::unwrap_used,
    reason = "constant regexes are exercised by unit tests"
)]
static PYTEST: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?m)^=+ (?P<body>.*\d+ (?:passed|failed).*) in [\d.]+s(?: \([^)]*\))? =+\s*$",
    )
    .unwrap()
});
#[expect(
    clippy::unwrap_used,
    reason = "constant regexes are exercised by unit tests"
)]
static TALLY: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(?P<count>\d+) (?P<name>passed|failed)").unwrap());

fn count(captures: &regex::Captures<'_>, name: &str) -> u64 {
    captures
        .name(name)
        .and_then(|value| value.as_str().parse().ok())
        .unwrap_or(0)
}

/// Test totals from the summary lines common runners print: cargo (summed
/// over every test binary), pytest, and jest/vitest. Other output yields none,
/// so a command that merely mentions "passed" is not read as a test run.
fn test_tally(text: &str) -> Option<ShellOutcome> {
    let text = ANSI.replace_all(text, "");

    let mut cargo = None::<(u64, u64)>;
    for captures in CARGO.captures_iter(&text) {
        let (passed, failed) = cargo.unwrap_or_default();
        cargo = Some((
            passed + count(&captures, "passed"),
            failed + count(&captures, "failed"),
        ));
    }
    if let Some((passed, failed)) = cargo.filter(|(passed, failed)| passed + failed > 0) {
        return Some(ShellOutcome::Tests { passed, failed });
    }

    // jest "Tests:  2 failed, 10 passed, 12 total"; vitest "Tests  2 failed | 10 passed (12)".
    if let Some(line) = JEST
        .captures_iter(&text)
        .last()
        .and_then(|captures| captures.name("body"))
        && let Some(tally) = tally_line(line.as_str())
    {
        return Some(tally);
    }

    // pytest "==== 3 failed, 9 passed in 0.12s ====" (the last summary wins).
    PYTEST
        .captures_iter(&text)
        .last()
        .and_then(|captures| captures.name("body"))
        .and_then(|line| tally_line(line.as_str()))
}

fn tally_line(line: &str) -> Option<ShellOutcome> {
    let pick = |name: &str| -> u64 {
        TALLY
            .captures_iter(line)
            .filter(|captures| captures.name("name").map(|n| n.as_str()) == Some(name))
            .map(|captures| count(&captures, "count"))
            .sum()
    };
    let (passed, failed) = (pick("passed"), pick("failed"));
    (passed + failed > 0).then_some(ShellOutcome::Tests { passed, failed })
}

#[derive(Debug, PartialEq, Eq)]
struct GrepOutcome {
    total: u64,
    /// Files with a match, known only when every match was returned.
    files: Option<usize>,
}

impl GrepOutcome {
    fn phrase(&self, pattern: Option<&str>, uk: bool) -> String {
        let pattern = pattern.map(|pattern| format!("`{pattern}`"));
        if self.total == 0 {
            return match (pattern, uk) {
                (Some(pattern), false) => format!("No matches for {pattern}"),
                (None, false) => "No matches".to_string(),
                (Some(pattern), true) => format!("Не знайшов {pattern} у файлах"),
                (None, true) => "Нічого не знайшов у файлах".to_string(),
            };
        }
        if uk {
            let mut line = format!("Знайшов збігів: {}", self.total);
            if let Some(pattern) = pattern {
                line = format!("Знайшов {pattern}, збігів: {}", self.total);
            }
            if let Some(files) = self.files {
                line.push_str(&format!(", файлів: {files}"));
            }
            return line;
        }
        let mut line = if self.total == 1 {
            "Found 1 match".to_string()
        } else {
            format!("Found {} matches", self.total)
        };
        if let Some(pattern) = pattern {
            line.push_str(&format!(" for {pattern}"));
        }
        match self.files {
            Some(1) => line.push_str(" in 1 file"),
            Some(files) => line.push_str(&format!(" in {files} files")),
            None => {}
        }
        line
    }
}

fn grep_outcome(result: &Value) -> Option<GrepOutcome> {
    let total = result
        .get("total_matches")
        .or_else(|| result.get("match_count"))
        .and_then(Value::as_u64)?;
    let returned = result.get("match_count").and_then(Value::as_u64);
    let paths: BTreeSet<&str> = ["matches", "blocks"]
        .iter()
        .filter_map(|key| result.get(*key).and_then(Value::as_array))
        .flatten()
        .filter_map(|entry| entry.get("path").and_then(Value::as_str))
        .collect();
    let complete = returned == Some(total) && (total == 0 || !paths.is_empty());
    Some(GrepOutcome {
        total,
        files: complete.then_some(paths.len()).filter(|files| *files > 0),
    })
}

#[cfg(test)]
#[path = "outcome_tests.rs"]
mod tests;
