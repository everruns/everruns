use serde_json::{Value, json};

use super::*;

fn shell_done(result: &Value, locale: Option<&str>) -> String {
    narrate_shell_exec(
        &json!({ "command": "cargo test" }),
        "Shell",
        ToolNarrationPhase::Completed,
        locale,
        ToolNarrationContext::default().with_result(Some(result)),
    )
}

fn grep_done(pattern: Option<&str>, result: &Value, locale: Option<&str>) -> String {
    let arguments = match pattern {
        Some(pattern) => json!({ "pattern": pattern }),
        None => json!({}),
    };
    narrate_grep_files(
        &arguments,
        ToolNarrationPhase::Completed,
        locale,
        ToolNarrationContext::default().with_result(Some(result)),
    )
}

#[test]
fn cargo_test_totals_are_summed_over_every_test_binary() {
    let stdout = "running 3 tests\n...\ntest result: ok. 3 passed; 0 failed; 1 ignored; \
                  0 measured; 0 filtered out\n\nrunning 9 tests\ntest result: FAILED. 6 passed; \
                  3 failed; 0 ignored; 0 measured; 0 filtered out\n";
    let result = json!({ "stdout": stdout, "stderr": "", "exit_code": 101 });
    assert_eq!(
        shell_done(&result, None),
        "Ran `cargo test`, 3 of 12 tests failed"
    );
    assert_eq!(
        shell_done(&result, Some("uk")),
        "Запустив `cargo test`, не пройшли тести: 3 з 12"
    );

    let passing =
        json!({ "stdout": "test result: ok. 12 passed; 0 failed; 0 ignored", "exit_code": 0 });
    assert_eq!(
        shell_done(&passing, None),
        "Ran `cargo test`, 12 tests passed"
    );
    assert_eq!(
        shell_done(&passing, Some("uk")),
        "Запустив `cargo test`, тестів пройдено: 12"
    );
}

#[test]
fn pytest_jest_and_vitest_summaries_are_read() {
    let pytest = json!({ "stdout": "tests/test_a.py ..F\n\
        ======= 1 failed, 9 passed, 2 skipped in 0.42s =======\n", "exit_code": 1 });
    assert_eq!(
        shell_done(&pytest, None),
        "Ran `cargo test`, 1 of 10 tests failed"
    );

    let jest = json!({ "stderr": "Test Suites: 1 failed, 3 passed, 4 total\n\
        Tests:       2 failed, 10 passed, 12 total\nSnapshots:   0 total\n", "exit_code": 1 });
    assert_eq!(
        shell_done(&jest, None),
        "Ran `cargo test`, 2 of 12 tests failed"
    );

    // vitest colours its summary; the escape codes are not part of the count.
    let vitest = json!({ "stdout": " Test Files  4 passed (4)\n      \u{1b}[2mTests\u{1b}[22m  \
        \u{1b}[1m\u{1b}[32m31 passed\u{1b}[39m\u{1b}[22m (31)\n", "exit_code": 0 });
    assert_eq!(
        shell_done(&vitest, None),
        "Ran `cargo test`, 31 tests passed"
    );

    let one = json!("==== 1 passed in 0.01s ====");
    assert_eq!(shell_done(&one, None), "Ran `cargo test`, 1 test passed");
}

#[test]
fn without_a_test_summary_only_a_non_zero_exit_is_reported() {
    let failed = json!({ "stdout": "", "stderr": "error[E0425]", "exit_code": 101 });
    assert_eq!(shell_done(&failed, None), "Ran `cargo test`, exit code 101");
    assert_eq!(
        shell_done(&failed, Some("uk")),
        "Запустив `cargo test`, код виходу 101"
    );

    // Output that merely mentions the words is not a test run.
    let chatter = json!({ "stdout": "3 passed the review; 1 failed to reply", "exit_code": 0 });
    assert_eq!(shell_done(&chatter, None), "Ran `cargo test`");
    assert_eq!(shell_done(&json!(null), None), "Ran `cargo test`");
}

#[test]
fn only_the_completed_phase_reads_the_result() {
    let result = json!({ "stdout": "test result: ok. 1 passed; 0 failed", "exit_code": 0 });
    for (phase, expected) in [
        (ToolNarrationPhase::Started, "Running `cargo test`"),
        (ToolNarrationPhase::Failed, "Failed to run `cargo test`"),
    ] {
        assert_eq!(
            narrate_shell_exec(
                &json!({ "command": "cargo test" }),
                "Shell",
                phase,
                None,
                ToolNarrationContext::default().with_result(Some(&result)),
            ),
            expected
        );
    }
}

#[test]
fn grep_counts_matches_and_the_files_they_are_in() {
    let result = json!({
        "pattern": "parse", "match_count": 3, "total_matches": 3,
        "matches": [
            { "path": "src/a.rs", "line_number": 1, "line": "parse" },
            { "path": "src/a.rs", "line_number": 9, "line": "parse" },
            { "path": "src/b.rs", "line_number": 2, "line": "parse" },
        ]
    });
    assert_eq!(
        grep_done(Some("parse"), &result, None),
        "Found 3 matches for `parse` in 2 files"
    );
    assert_eq!(
        grep_done(Some("parse"), &result, Some("uk")),
        "Знайшов `parse`, збігів: 3, файлів: 2"
    );

    let blocks = json!({
        "match_count": 1, "total_matches": 1,
        "blocks": [{ "path": "src/a.rs", "start_line": 1, "end_line": 3 }]
    });
    assert_eq!(grep_done(None, &blocks, None), "Found 1 match in 1 file");
}

#[test]
fn a_truncated_grep_reports_the_total_without_guessing_files() {
    let result = json!({
        "match_count": 2, "total_matches": 40,
        "matches": [
            { "path": "src/a.rs", "line_number": 1, "line": "x" },
            { "path": "src/b.rs", "line_number": 1, "line": "x" },
        ]
    });
    assert_eq!(
        grep_done(Some("x"), &result, None),
        "Found 40 matches for `x`"
    );
}

#[test]
fn an_empty_grep_says_nothing_matched() {
    let result = json!({ "match_count": 0, "total_matches": 0, "matches": [] });
    assert_eq!(
        grep_done(Some("nope"), &result, None),
        "No matches for `nope`"
    );
    assert_eq!(
        grep_done(Some("nope"), &result, Some("uk")),
        "Не знайшов `nope` у файлах"
    );
    // A result without counts keeps the plain line.
    assert_eq!(
        grep_done(Some("nope"), &json!({ "error": "x" }), None),
        "Searched files for nope"
    );
}
