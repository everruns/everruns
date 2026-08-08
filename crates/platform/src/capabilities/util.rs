//! Shared helpers for hosted platform-management tools (EVE-839).
//!
//! The generic argument parsers are reused from `everruns-core`;
//! [`get_platform_store`] resolves the hosted `PlatformStore` from the
//! tool-context extension that the host installs for hosted deployments.

pub use everruns_core::capabilities::util::{get_str, parse_id, require_id, require_str};

use crate::platform_store::{PlatformStore, PlatformStoreExt};
use everruns_core::tools::ToolExecutionResult;
use everruns_core::traits::ToolContext;
use std::sync::Arc;

/// Resolve the hosted platform store from the tool-context extension, or return
/// the existing model-visible configuration error (behavior preserved from the
/// former core `get_platform_store`).
pub fn get_platform_store(
    context: &ToolContext,
) -> Result<Arc<dyn PlatformStore>, ToolExecutionResult> {
    context
        .extension::<PlatformStoreExt>()
        .map(|ext| ext.0.clone())
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(
                "Platform management not available: platform_store context is missing. Ensure the platform_management capability is enabled.",
            )
        })
}
