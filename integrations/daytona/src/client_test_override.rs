//! Test-only hook for redirecting `DaytonaApiCallTool` at a mock server.
//!
//! `DaytonaClient::new` always points at the real Daytona endpoints, and
//! `daytona_api_call`'s request schema (method/path/body) has no field a
//! caller could use to redirect it. Everything else in this crate that
//! needs a test double calls `DaytonaClient::with_base_urls` directly from
//! its own test, but `daytona_api_call`'s tool-level tests (label injection,
//! lease tracking) need the full `execute_with_context` path, HTTP call
//! included. This attaches the override via `ToolContext`'s generic,
//! type-erased extension bag rather than adding a base-URL parameter to the
//! tool's public request schema.
//!
//! Decision: the override only exists under the `test-util` feature, which
//! only this crate's own dev-dependency enables. Shipped builds always use
//! the real endpoints, so nothing at runtime can send the API key elsewhere.

use everruns_core::tool_context::ToolContext;

use crate::client::DaytonaClient;

/// Test-only override for the Daytona API base URLs, attached to
/// [`ToolContext`] extensions. See the module docs above.
#[cfg(feature = "test-util")]
#[derive(Debug, Clone)]
pub struct DaytonaBaseUrlOverride {
    pub api_base: String,
    pub toolbox_base: String,
}

/// Build a `DaytonaClient` for `context`. With `test-util`, honors a
/// `DaytonaBaseUrlOverride` extension; otherwise always the real endpoints.
pub fn daytona_client_from_context(api_key: String, context: &ToolContext) -> DaytonaClient {
    #[cfg(feature = "test-util")]
    if let Some(o) = context.extension::<DaytonaBaseUrlOverride>() {
        return DaytonaClient::with_base_urls(api_key, o.api_base.clone(), o.toolbox_base.clone());
    }
    #[cfg(not(feature = "test-util"))]
    let _ = context;
    DaytonaClient::new(api_key)
}
