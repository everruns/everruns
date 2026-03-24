//! PI agent state persistence helpers.

use everruns_core::UpsertLeasedResource;
use everruns_core::tools::ToolExecutionResult;
use everruns_core::traits::ToolContext;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{error, warn};

use crate::{PI_AGENT_SECRET_PREFIX, PI_DEFAULT_TIMEOUT_SECS};

/// Persisted state for a running PI agent instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiAgentState {
    pub agent_id: String,
    pub sandbox_id: String,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub model: String,
    pub status: PiAgentStatus,
    pub task: String,
    pub started_at: String,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PiAgentStatus {
    Starting,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl std::fmt::Display for PiAgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Starting => write!(f, "starting"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

pub const PI_AGENT_LEASE_DURATION_SECONDS: u32 = 30 * 60;

pub fn required_str<'a>(
    args: &'a serde_json::Value,
    name: &str,
) -> Result<&'a str, ToolExecutionResult> {
    args.get(name)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!("Missing required parameter: {name}"))
        })
}

pub async fn get_agent_state(
    context: &ToolContext,
    agent_id: &str,
) -> Result<PiAgentState, ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let secret_name = format!("{PI_AGENT_SECRET_PREFIX}{agent_id}");
    let json_str = storage
        .get_secret(context.session_id, &secret_name)
        .await
        .map_err(|e| {
            error!("Failed to read PI agent state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to read agent state: {e}"))
        })?
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!(
                "PI agent '{agent_id}' not found. Start one first with start_pi."
            ))
        })?;

    serde_json::from_str(&json_str).map_err(|e| {
        error!("Corrupt PI agent state for {agent_id}: {e}");
        ToolExecutionResult::internal_error_msg(format!("Corrupt agent state: {e}"))
    })
}

pub async fn save_agent_state(
    context: &ToolContext,
    state: &PiAgentState,
) -> Result<(), ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let secret_name = format!("{PI_AGENT_SECRET_PREFIX}{}", state.agent_id);
    let json_str = serde_json::to_string(state).map_err(|e| {
        ToolExecutionResult::internal_error_msg(format!("Failed to serialize agent state: {e}"))
    })?;

    storage
        .set_secret(context.session_id, &secret_name, &json_str)
        .await
        .map_err(|e| {
            error!("Failed to save PI agent state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to save agent state: {e}"))
        })
}

pub async fn delete_agent_state(
    context: &ToolContext,
    agent_id: &str,
) -> Result<(), ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let secret_name = format!("{PI_AGENT_SECRET_PREFIX}{agent_id}");
    storage
        .delete_secret(context.session_id, &secret_name)
        .await
        .map(|_| ())
        .map_err(|e| {
            error!("Failed to delete PI agent state: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to delete agent state: {e}"))
        })
}

pub async fn list_agent_states(
    context: &ToolContext,
) -> Result<Vec<PiAgentState>, ToolExecutionResult> {
    let storage = context
        .storage_store
        .as_ref()
        .ok_or_else(|| ToolExecutionResult::tool_error("Storage not available in this context"))?;

    let secrets = storage
        .list_secrets(context.session_id)
        .await
        .map_err(|e| {
            error!("Failed to list PI secrets: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to list secrets: {e}"))
        })?;

    let mut states = Vec::new();
    for secret_info in secrets {
        if let Some(agent_id) = secret_info.name.strip_prefix(PI_AGENT_SECRET_PREFIX) {
            match get_agent_state(context, agent_id).await {
                Ok(state) => states.push(state),
                Err(_) => warn!("Skipping corrupt PI agent state: {agent_id}"),
            }
        }
    }

    Ok(states)
}

pub async fn touch_agent_lease(
    context: &ToolContext,
    state: &PiAgentState,
    display_name: Option<String>,
) -> Result<(), ToolExecutionResult> {
    let Some(store) = context.leased_resource_store.as_ref() else {
        return Ok(());
    };

    store
        .upsert_resource(UpsertLeasedResource {
            session_id: context.session_id,
            provider: "pi".to_string(),
            resource_type: "agent".to_string(),
            external_id: state.agent_id.clone(),
            display_name,
            owner_user_id: None,
            lease_duration_seconds: PI_AGENT_LEASE_DURATION_SECONDS,
            metadata: json!({
                "sandbox_id": state.sandbox_id,
                "repo": state.repo,
                "branch": state.branch,
                "model": state.model,
                "status": state.status.to_string(),
                "started_at": state.started_at,
            }),
        })
        .await
        .map_err(|e| {
            error!("Failed to upsert PI agent lease: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to update agent lease: {e}"))
        })?;

    Ok(())
}

pub async fn release_agent_lease(
    context: &ToolContext,
    agent_id: &str,
) -> Result<(), ToolExecutionResult> {
    let Some(store) = context.leased_resource_store.as_ref() else {
        return Ok(());
    };

    store
        .release_resource(context.session_id, "pi", "agent", agent_id)
        .await
        .map_err(|e| {
            error!("Failed to release PI agent lease: {e}");
            ToolExecutionResult::internal_error_msg(format!("Failed to release agent lease: {e}"))
        })?;

    Ok(())
}

pub fn new_agent_id() -> String {
    format!("pi_{}", uuid::Uuid::now_v7().simple())
}

pub fn parse_timeout_seconds(arguments: &serde_json::Value) -> Result<u64, ToolExecutionResult> {
    match arguments.get("timeout_seconds") {
        None => Ok(PI_DEFAULT_TIMEOUT_SECS),
        Some(value) => value
            .as_u64()
            .filter(|timeout| *timeout >= 60)
            .ok_or_else(|| {
                ToolExecutionResult::tool_error(
                    "Invalid 'timeout_seconds': must be an integer >= 60",
                )
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_state_roundtrip() {
        let state = PiAgentState {
            agent_id: "pi_test123".to_string(),
            sandbox_id: "sb_abc".to_string(),
            repo: Some("owner/repo".to_string()),
            branch: Some("main".to_string()),
            model: "claude-sonnet-4-20250514".to_string(),
            status: PiAgentStatus::Running,
            task: "Fix the auth bug".to_string(),
            started_at: "2026-03-24T00:00:00Z".to_string(),
            timeout_seconds: 3600,
        };

        let json = serde_json::to_string(&state).unwrap();
        let decoded: PiAgentState = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.agent_id, "pi_test123");
        assert_eq!(decoded.sandbox_id, "sb_abc");
        assert_eq!(decoded.model, "claude-sonnet-4-20250514");
        assert_eq!(decoded.status, PiAgentStatus::Running);
        assert_eq!(decoded.timeout_seconds, 3600);
    }

    #[test]
    fn status_display() {
        assert_eq!(PiAgentStatus::Starting.to_string(), "starting");
        assert_eq!(PiAgentStatus::Running.to_string(), "running");
        assert_eq!(PiAgentStatus::Completed.to_string(), "completed");
        assert_eq!(PiAgentStatus::Failed.to_string(), "failed");
        assert_eq!(PiAgentStatus::Cancelled.to_string(), "cancelled");
    }
}
