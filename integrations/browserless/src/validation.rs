//! Browserless URL validation helpers.
//!
//! Decision: Reuse everruns-core SSRF validation for all user-provided Browserless
//! URLs so Browserless tools match the threat model and share one policy with MCP
//! and provider base URL validation.

use everruns_core::tools::ToolExecutionResult;
use serde_json::Value;

/// THREAT[TM-TOOL-015]: Block Browserless navigation to internal/private hosts.
/// Mitigation: Reuse shared URL validation so localhost, RFC1918, link-local,
/// and metadata endpoints are rejected before any Browserless API call.
pub(crate) fn validate_browserless_url(url: &str) -> Result<(), ToolExecutionResult> {
    everruns_core::validate_safe_url(url)
        .map(|_| ())
        .map_err(|e| ToolExecutionResult::tool_error(format!("URL is blocked by policy: {e}")))
}

/// THREAT[TM-TOOL-015]: Validate all nested navigate actions in interaction steps.
/// Mitigation: Reject Browserless interact payloads that attempt to navigate to
/// blocked hosts after the initial page load.
pub(crate) fn validate_interaction_steps(steps: &[Value]) -> Result<(), ToolExecutionResult> {
    for step in steps {
        if step.get("action").and_then(|v| v.as_str()) == Some("navigate") {
            let url = step
                .get("value")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolExecutionResult::tool_error("navigate requires value (URL)"))?;
            validate_browserless_url(url)?;
        }
    }

    Ok(())
}
