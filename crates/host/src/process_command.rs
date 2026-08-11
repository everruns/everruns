//! Opt-in host-process command executor for trusted skill preprocessing.

use everruns_core::skill::{CommandExecutor, CommandResult};

/// Execute trusted preprocessing commands through the host's `bash` process.
///
/// This implementation is intentionally behind `everruns-host/process`; the
/// neutral kernel and default Framework do not compile or install it.
pub struct ProcessCommandExecutor {
    /// Timeout per command in seconds (default: 30).
    pub timeout_secs: u64,
}

impl Default for ProcessCommandExecutor {
    fn default() -> Self {
        Self { timeout_secs: 30 }
    }
}

#[async_trait::async_trait]
impl CommandExecutor for ProcessCommandExecutor {
    async fn execute_command(&self, command: &str) -> CommandResult {
        let timeout = std::time::Duration::from_secs(self.timeout_secs);
        let mut process = tokio::process::Command::new("bash");
        process
            .arg("-c")
            .arg(command)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let child = process.spawn();

        let child = match child {
            Ok(child) => child,
            Err(_) => {
                return CommandResult {
                    stdout: String::new(),
                    exit_code: -1,
                };
            }
        };

        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(output)) => CommandResult {
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                exit_code: output.status.code().unwrap_or(-1),
            },
            Ok(Err(_)) => CommandResult {
                stdout: String::new(),
                exit_code: -1,
            },
            Err(_) => CommandResult {
                stdout: format!(
                    "[Command timed out after {}s: {command}]",
                    self.timeout_secs
                ),
                exit_code: -1,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn executes_trusted_preprocessing_command() {
        let result = ProcessCommandExecutor::default()
            .execute_command("printf 'hello world'")
            .await;

        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout, "hello world");
    }

    #[tokio::test]
    async fn timeout_returns_failure_and_cancels_child() {
        let executor = ProcessCommandExecutor { timeout_secs: 1 };
        let started = tokio::time::Instant::now();
        let result = executor.execute_command("sleep 5").await;

        assert_eq!(result.exit_code, -1);
        assert!(result.stdout.contains("timed out after 1s"));
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
    }
}
