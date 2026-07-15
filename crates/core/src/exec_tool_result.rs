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
    use crate::tool_output_sanitizer::{AUTO_SUCCESS_BUDGET, NORMAL_BUDGET};

    #[test]
    fn preserves_full_raw_output_and_marks_truncation() {
        let stdout = (0..400)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let payload = ExecToolResultPayload::new(&stdout, "warn\n", 17, "concise");

        assert_eq!(payload.exit_code, 17);
        assert!(!payload.success);
        assert!(payload.truncated);
        assert_eq!(payload.total_lines, 400);
        assert!(payload.stdout.len() < stdout.len());
        assert!(payload.raw_output.contains("line 0"));
        assert!(payload.raw_output.contains("line 399"));
        assert!(payload.raw_output.contains("--- stderr ---"));
    }

    #[test]
    fn full_mode_keeps_complete_output_inline() {
        let payload = ExecToolResultPayload::new("alpha\nbeta\n", "", 0, "full");

        assert_eq!(payload.stdout, "alpha\nbeta\n");
        assert_eq!(payload.stderr, "");
        assert!(!payload.truncated);
        assert_eq!(payload.total_lines, 2);
        assert_eq!(payload.raw_output, "alpha\nbeta\n");
    }

    // ====================================================================
    // EVE-489: auto mode is persistence-first
    // ====================================================================

    #[test]
    fn auto_success_produces_compact_inline_output() {
        let stdout = (0..2000)
            .map(|i| format!("success-line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let payload = ExecToolResultPayload::new(&stdout, "", 0, "auto");

        assert!(payload.success);
        assert!(
            payload.stdout.len() <= AUTO_SUCCESS_BUDGET,
            "auto+success stdout should fit in AUTO_SUCCESS_BUDGET ({}), got {} bytes",
            AUTO_SUCCESS_BUDGET,
            payload.stdout.len()
        );
        // total_lines is computed from cleaned (untruncated) stdout
        assert_eq!(payload.total_lines, 2000);
        // raw_output still contains every line for the persistence hook
        assert!(payload.raw_output.contains("success-line-0"));
        assert!(payload.raw_output.contains("success-line-1999"));
        assert!(payload.truncated);
    }

    #[test]
    fn auto_failure_uses_normal_diagnostic_budget() {
        // Build output large enough that NORMAL_BUDGET truncation kicks in
        // but the result still has substantial diagnostic detail.
        let mut lines = Vec::new();
        for i in 0..200 {
            lines.push(format!("building module {i}"));
        }
        lines.push("error: failed to compile".to_string());
        lines.push("  --> src/main.rs:1:1".to_string());
        for i in 0..2000 {
            lines.push(format!("trailing diagnostic line {i}"));
        }
        let stdout = lines.join("\n");

        let payload = ExecToolResultPayload::new(&stdout, "stderr details\n", 1, "auto");

        assert!(!payload.success);
        assert_eq!(payload.exit_code, 1);
        // Auto + failure should give a diagnostic window, not the tiny success budget.
        assert!(
            payload.stdout.len() > AUTO_SUCCESS_BUDGET,
            "auto+failure stdout should exceed AUTO_SUCCESS_BUDGET to fit diagnostics, got {} bytes",
            payload.stdout.len()
        );
        assert!(
            payload.stdout.len() <= NORMAL_BUDGET,
            "auto+failure stdout should be capped at NORMAL_BUDGET ({}), got {} bytes",
            NORMAL_BUDGET,
            payload.stdout.len()
        );
        // Error region is preserved through priority-aware truncation.
        assert!(
            payload.stdout.contains("error: failed to compile"),
            "error line must be preserved in failure diagnostic output"
        );
        // raw_output still has the full content for persistence.
        assert!(payload.raw_output.contains("building module 0"));
        assert!(payload.raw_output.contains("trailing diagnostic line 1999"));
        assert!(payload.raw_output.contains("--- stderr ---"));
    }

    #[test]
    fn auto_small_success_output_is_unchanged() {
        let payload = ExecToolResultPayload::new("ok\n", "", 0, "auto");
        assert!(payload.success);
        assert_eq!(payload.stdout, "ok\n");
        assert!(!payload.truncated);
    }

    #[test]
    fn explicit_normal_still_uses_normal_budget_on_success() {
        let stdout = (0..2000)
            .map(|i| format!("line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let payload = ExecToolResultPayload::new(&stdout, "", 0, "normal");

        assert!(payload.success);
        // Normal on success returns NORMAL_BUDGET worth of inline output,
        // not the auto-success compact summary.
        assert!(
            payload.stdout.len() > AUTO_SUCCESS_BUDGET,
            "explicit normal mode should not compact to auto-success budget on success"
        );
        assert!(payload.stdout.len() <= NORMAL_BUDGET);
    }

    #[test]
    fn successful_source_search_preserves_leading_matches() {
        let mut lines = vec![
            "src/runtime.rs:10: first relevant match".to_string(),
            "src/runtime.rs:11: second relevant match".to_string(),
        ];
        lines.extend((0..1200).map(|i| format!("src/module_{i}.rs: ordinary source line")));
        lines.extend((0..300).map(|i| format!("src/errors_{i}.rs: struct ErrorContext{i}")));
        let stdout = lines.join("\n");

        let payload = ExecToolResultPayload::new(&stdout, "", 0, "normal");

        assert!(payload.truncated);
        assert!(payload.stdout.contains("first relevant match"));
        assert!(payload.stdout.contains("second relevant match"));
    }

    #[test]
    fn raw_output_preserved_across_modes() {
        let stdout = (0..5000)
            .map(|i| format!("line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");

        for mode in ["auto", "silent", "concise", "normal", "verbose", "full"] {
            for exit_code in [0, 1] {
                let payload = ExecToolResultPayload::new(&stdout, "err\n", exit_code, mode);
                assert!(
                    payload.raw_output.contains("line-0"),
                    "raw_output should contain head for mode={mode} exit={exit_code}"
                );
                assert!(
                    payload.raw_output.contains("line-4999"),
                    "raw_output should contain tail for mode={mode} exit={exit_code}"
                );
                assert!(
                    payload.raw_output.contains("--- stderr ---"),
                    "raw_output should contain stderr marker for mode={mode} exit={exit_code}"
                );
            }
        }
    }
}
