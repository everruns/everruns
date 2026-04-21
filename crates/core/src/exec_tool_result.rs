//! Shared shaping helpers for human-facing exec tool results.
//!
//! Important decision: keep the visible JSON contract stable across shell-like
//! tools and keep pre-truncation output in the raw sidecar only. UI cards,
//! previews, persistence hooks, and narration all depend on this split.

use crate::tool_output_sanitizer::{
    clean_exec_output, output_verbosity_budget, priority_aware_truncate,
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
        let (stdout, stderr) = if let Some(budget) = output_verbosity_budget(output_mode) {
            (
                priority_aware_truncate(&clean_stdout, budget),
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
}
