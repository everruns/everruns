//! Shared shaping helpers for human-facing exec tool results.
//!
//! Important decision: keep the visible JSON contract stable across shell-like
//! tools and keep pre-truncation output in the raw sidecar only. UI cards,
//! previews, persistence hooks, and narration all depend on this split.

use crate::tool_output_sanitizer::{
    clean_exec_output, output_verbosity_budget, priority_aware_truncate, resolve_auto_mode,
    truncate_exec_stream,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecToolResultPayload {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub success: bool,
    pub truncated: bool,
    pub total_lines: usize,
    pub raw_output: String,
}

impl ExecToolResultPayload {
    pub fn new(stdout: &str, stderr: &str, exit_code: i32, output_mode: &str) -> Self {
        let clean_stdout = clean_exec_output(stdout);
        let clean_stderr = clean_exec_output(stderr);
        // EVE-489: `auto` is persistence-first — successful persisted exec
        // calls return a tiny inline summary so the model can rely on the
        // persisted /outputs files for the full log, while failures fall back
        // to a `normal`-sized window for in-loop debugging.
        let effective_mode = resolve_auto_mode(output_mode, exit_code);
        let (stdout, stderr) = if let Some(budget) = output_verbosity_budget(effective_mode) {
            (
                truncate_exec_stream(&clean_stdout, budget, exit_code),
                priority_aware_truncate(&clean_stderr, budget.min(4096)),
            )
        } else {
            (clean_stdout.clone(), clean_stderr.clone())
        };
        let truncated = stdout != clean_stdout || stderr != clean_stderr;
        let total_lines = clean_stdout.lines().count();
        let mut raw_output = clean_stdout;
        if !clean_stderr.is_empty() {
            raw_output.push_str("\n--- stderr ---\n");
            raw_output.push_str(&clean_stderr);
        }

        Self {
            stdout,
            stderr,
            exit_code,
            success: exit_code == 0,
            truncated,
            total_lines,
            raw_output,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ExecToolResultPayload;

    #[test]
    fn complete_payload_cleans_streams_before_counting_and_preserving_raw_output() {
        for mode in ["full", "auto", "concise"] {
            assert_eq!(
                ExecToolResultPayload::new(
                    "\u{1b}[31malpha\u{1b}[0m\r\nbeta\n",
                    "old\rwarning\n",
                    17,
                    mode
                ),
                ExecToolResultPayload {
                    stdout: "alpha\nbeta\n".into(),
                    stderr: "warning\n".into(),
                    exit_code: 17,
                    success: false,
                    truncated: false,
                    total_lines: 2,
                    raw_output: "alpha\nbeta\n\n--- stderr ---\nwarning\n".into(),
                }
            );
            assert_eq!(
                ExecToolResultPayload::new("ok\n", "", 0, mode),
                ExecToolResultPayload {
                    stdout: "ok\n".into(),
                    stderr: String::new(),
                    exit_code: 0,
                    success: true,
                    truncated: false,
                    total_lines: 1,
                    raw_output: "ok\n".into(),
                }
            );
        }
    }

    #[test]
    fn literal_inline_budgets_accept_boundary_and_truncate_one_byte_over() {
        for (mode, exit, budget) in [
            ("silent", 0, 200),
            ("concise", 0, 2048),
            ("normal", 0, 8192),
            ("verbose", 0, 16384),
            ("auto", 0, 384),
            ("auto", 1, 8192),
            ("unknown", 0, 2048),
        ] {
            let exact = "x".repeat(budget);
            let payload = ExecToolResultPayload::new(&exact, "", exit, mode);
            assert_eq!(payload.stdout, exact, "{mode}/{exit}");
            assert!(!payload.truncated);
            let over = "x".repeat(budget + 1);
            let payload = ExecToolResultPayload::new(&over, "", exit, mode);
            assert!(payload.truncated, "{mode}/{exit}");
            assert!(payload.stdout.len() <= budget, "{mode}/{exit}");
            assert_eq!(payload.raw_output, over);
            assert_eq!(payload.total_lines, 1);
            assert_eq!(payload.success, exit == 0);
        }
    }

    #[test]
    fn every_mode_preserves_complete_raw_streams_and_caps_stderr() {
        let stdout = (0..1800)
            .map(|i| format!("line {i:04}\n"))
            .collect::<String>();
        let stderr = (0..700)
            .map(|i| format!("warning {i:03}\n"))
            .collect::<String>();
        let raw = format!("{stdout}\n--- stderr ---\n{stderr}");
        for mode in ["auto", "silent", "concise", "normal", "verbose", "full"] {
            for exit in [0, 17] {
                let payload = ExecToolResultPayload::new(&stdout, &stderr, exit, mode);
                assert_eq!(payload.raw_output, raw, "{mode}/{exit}");
                assert_eq!(payload.total_lines, 1800);
                assert_eq!(payload.exit_code, exit);
                assert_eq!(payload.success, exit == 0);
                if mode == "full" {
                    assert_eq!(payload.stdout, stdout);
                    assert_eq!(payload.stderr, stderr);
                    assert!(!payload.truncated);
                } else {
                    assert!(payload.truncated);
                    assert!(payload.stderr.len() <= 4096);
                }
            }
        }
    }

    #[test]
    fn auto_failure_preserves_middle_diagnostics_with_normal_budget() {
        let stdout = format!(
            "{}error: failed to compile\n  --> src/main.rs:1:1\n{}",
            "building module\n".repeat(300),
            "trailing diagnostic line\n".repeat(600)
        );
        let payload = ExecToolResultPayload::new(&stdout, "stderr details\n", 1, "auto");
        assert!(!payload.success);
        assert_eq!(payload.exit_code, 1);
        assert!(payload.truncated);
        assert!(payload.stdout.len() > 384 && payload.stdout.len() <= 8192);
        assert!(
            payload
                .stdout
                .contains("error: failed to compile\n  --> src/main.rs:1:1")
        );
        assert_eq!(
            payload.raw_output,
            format!("{stdout}\n--- stderr ---\nstderr details\n")
        );
    }

    #[test]
    fn successful_source_search_preserves_leading_matches() {
        let prefix =
            "src/runtime.rs:10: first relevant match\nsrc/runtime.rs:11: second relevant match\n";
        let stdout = format!(
            "{prefix}{}{}",
            "src/module.rs: ordinary source line\n".repeat(400),
            "src/errors.rs: struct ErrorContext\n".repeat(100)
        );
        let payload = ExecToolResultPayload::new(&stdout, "", 0, "normal");
        assert!(payload.truncated);
        assert!(payload.stdout.starts_with(prefix));
        assert_eq!(payload.raw_output, stdout);
    }
}
