//! Carrying a host's own tool result through `anyhow`.
//!
//! A sandbox can answer with more than an error string: "connect your account
//! first", or an internal error the platform must not show the model. Those
//! results have to reach the caller unchanged, so the code in between (which
//! speaks `anyhow`) carries them as a [`Passthrough`] and the tool boundary
//! unwraps them again with [`into_tool_result`].

use everruns_contracts::runtime::tools::ToolExecutionResult;
use std::sync::Mutex;

/// A tool result travelling inside an [`anyhow::Error`].
pub(crate) struct Passthrough(Mutex<Option<ToolExecutionResult>>);

impl Passthrough {
    pub(crate) fn wrap(result: ToolExecutionResult) -> anyhow::Error {
        anyhow::Error::new(Self(Mutex::new(Some(result))))
    }
}

impl std::fmt::Debug for Passthrough {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Passthrough")
    }
}

impl std::fmt::Display for Passthrough {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0.lock() {
            Ok(guard) => match guard.as_ref() {
                Some(ToolExecutionResult::ToolError(message)) => f.write_str(message),
                _ => f.write_str("the sandbox could not complete the request"),
            },
            Err(_) => f.write_str("the sandbox could not complete the request"),
        }
    }
}

impl std::error::Error for Passthrough {}

/// The tool result for a failed call: the host's own result when one was
/// carried, otherwise the error text as a tool error the model can act on.
pub(crate) fn into_tool_result(error: anyhow::Error) -> ToolExecutionResult {
    for cause in error.chain() {
        if let Some(passthrough) = cause.downcast_ref::<Passthrough>()
            && let Ok(mut guard) = passthrough.0.lock()
            && let Some(result) = guard.take()
        {
            return result;
        }
    }
    ToolExecutionResult::tool_error(format!("{error:#}"))
}

/// Run parsing work off the async threads: one large scan must not stall the
/// other sessions this process serves.
pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> anyhow::Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| anyhow::anyhow!("code analysis task failed: {err}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carried_results_come_back_unchanged() {
        let error = Passthrough::wrap(ToolExecutionResult::internal_error_msg("boom"))
            .context("while reading sources");
        assert!(matches!(
            into_tool_result(error),
            ToolExecutionResult::InternalError(_)
        ));
    }

    #[test]
    fn plain_errors_become_tool_errors_with_their_context() {
        let error = anyhow::anyhow!("no such file").context("read main.rs");
        let ToolExecutionResult::ToolError(message) = into_tool_result(error) else {
            panic!("expected a tool error");
        };
        assert_eq!(message, "read main.rs: no such file");
    }
}
