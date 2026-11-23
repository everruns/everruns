//! Explicit redirection affects only calls not yet admitted by the scheduler.
//! Hosts retain the input until the next reason boundary. This port never
//! consumes input, interrupts running calls, or resolves approval requests.
use async_trait::async_trait;
use everruns_contracts::{error::Result, typed_id::TurnId};

/// Reserved input metadata. Its value is the exact turn the user redirects.
pub const REDIRECT_TURN: &str = "everruns.redirect_turn";

/// Read-only admission check, called after acquiring the scheduler permit.
/// Durable implementations must read retained input, not a process-local hint.
#[async_trait]
pub trait ToolBatchSteering: Send + Sync {
    async fn redirected(&self, turn_id: TurnId) -> Result<bool>;
}

pub fn targets(
    metadata: Option<&std::collections::HashMap<String, serde_json::Value>>,
    turn_id: TurnId,
) -> bool {
    metadata
        .and_then(|m| m.get(REDIRECT_TURN))
        .and_then(serde_json::Value::as_str)
        == Some(turn_id.to_string().as_str())
}
