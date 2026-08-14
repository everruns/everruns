//! Cursor Cloud Agents tool implementations.

use async_trait::async_trait;
use everruns_core::ToolHints;
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::{Tool, ToolExecutionResult};
use serde_json::{Value, json};
use tracing::{debug, error};

use crate::client::{CursorClient, LaunchAgentRequest};
use crate::{CURSOR_API_BASE_ENV, CURSOR_API_KEY_SECRET, CURSOR_CONNECTION_PROVIDER};

const MAX_PROMPT_CHARS: usize = 20_000;
const MAX_AGENT_ID_CHARS: usize = 128;
const MAX_REF_CHARS: usize = 255;
const MAX_BRANCH_CHARS: usize = 255;
const MAX_MODEL_CHARS: usize = 128;

async fn get_api_key(context: &ToolContext) -> Result<String, ToolExecutionResult> {
    if let Some(resolver) = context.connection_resolver.as_ref() {
        match resolver
            .get_connection_token(context.session_id, CURSOR_CONNECTION_PROVIDER)
            .await
        {
            Ok(Some(token)) if !token.trim().is_empty() => return Ok(token),
            Ok(_) => {}
            Err(e) => debug!("Cursor connection resolver failed: {e}"),
        }
    }

    if let Some(storage) = context.storage_store.as_ref() {
        match storage
            .get_secret(context.session_id, CURSOR_API_KEY_SECRET)
            .await
        {
            Ok(Some(key)) if !key.trim().is_empty() => return Ok(key),
            Ok(_) => {}
            Err(e) => {
                error!("Failed to read {CURSOR_API_KEY_SECRET} session secret: {e}");
                return Err(ToolExecutionResult::internal_error_msg(
                    "Failed to read Cursor API key",
                ));
            }
        }
    }

    // THREAT[TM-CURSOR-001]: Avoid asking for API keys in chat where they are
    // stored in plaintext messages. Signal the connection dialog instead.
    Err(ToolExecutionResult::connection_required(
        CURSOR_CONNECTION_PROVIDER,
    ))
}

fn cursor_client(api_key: String) -> CursorClient {
    match std::env::var(CURSOR_API_BASE_ENV) {
        Ok(api_base) if !api_base.trim().is_empty() => {
            CursorClient::with_base_url(api_key, api_base)
        }
        _ => CursorClient::new(api_key),
    }
}

fn required_str<'a>(arguments: &'a Value, name: &str) -> Result<&'a str, ToolExecutionResult> {
    arguments
        .get(name)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!("Missing required parameter: {name}"))
        })
}

fn optional_str<'a>(
    arguments: &'a Value,
    name: &str,
    max_chars: usize,
) -> Result<Option<&'a str>, ToolExecutionResult> {
    match arguments.get(name) {
        Some(Value::String(value)) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else if trimmed.chars().count() > max_chars {
                Err(ToolExecutionResult::tool_error(format!(
                    "Invalid '{name}': maximum length is {max_chars} characters"
                )))
            } else {
                Ok(Some(trimmed))
            }
        }
        Some(Value::Null) | None => Ok(None),
        Some(_) => Err(ToolExecutionResult::tool_error(format!(
            "Invalid '{name}': must be a string"
        ))),
    }
}

fn required_limited_str<'a>(
    arguments: &'a Value,
    name: &str,
    max_chars: usize,
) -> Result<&'a str, ToolExecutionResult> {
    let value = required_str(arguments, name)?;
    if value.chars().count() > max_chars {
        return Err(ToolExecutionResult::tool_error(format!(
            "Invalid '{name}': maximum length is {max_chars} characters"
        )));
    }
    Ok(value)
}

fn optional_bool(arguments: &Value, name: &str) -> Result<Option<bool>, ToolExecutionResult> {
    match arguments.get(name) {
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(Value::Null) | None => Ok(None),
        Some(_) => Err(ToolExecutionResult::tool_error(format!(
            "Invalid '{name}': must be a boolean"
        ))),
    }
}

fn optional_limit(arguments: &Value) -> Result<Option<u32>, ToolExecutionResult> {
    match arguments.get("limit") {
        Some(Value::Number(n)) => {
            let Some(value) = n.as_u64() else {
                return Err(ToolExecutionResult::tool_error(
                    "Invalid 'limit': must be an integer between 1 and 100",
                ));
            };
            if !(1..=100).contains(&value) {
                return Err(ToolExecutionResult::tool_error(
                    "Invalid 'limit': must be between 1 and 100",
                ));
            }
            Ok(Some(value as u32))
        }
        Some(Value::Null) | None => Ok(None),
        Some(_) => Err(ToolExecutionResult::tool_error(
            "Invalid 'limit': must be an integer between 1 and 100",
        )),
    }
}

fn context_required_error(name: &str) -> ToolExecutionResult {
    ToolExecutionResult::tool_error(format!(
        "{name} requires context. This tool must be executed with session context."
    ))
}

pub struct CursorLaunchAgentTool;

#[async_trait]
impl Tool for CursorLaunchAgentTool {
    fn name(&self) -> &str {
        "cursor_launch_agent"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Launch Cursor Agent")
    }

    fn description(&self) -> &str {
        "Start a Cursor Cloud Agent to work asynchronously on a GitHub repository."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": { "type": "string", "description": "Precise task instructions for the Cursor agent.", "minLength": 1, "maxLength": MAX_PROMPT_CHARS },
                "repository": { "type": "string", "description": "GitHub repository URL, e.g. https://github.com/org/repo.", "minLength": 1 },
                "ref": { "type": "string", "description": "Base branch, tag, or ref. Default is Cursor's repository default branch." },
                "model": { "type": "string", "description": "Optional Cursor model id. Omit for Cursor Auto." },
                "auto_create_pr": { "type": "boolean", "description": "Whether Cursor should create a pull request when the agent finishes. Default false." },
                "branch_name": { "type": "string", "description": "Optional custom branch name for Cursor to create." },
                "name": { "type": "string", "description": "Optional short human-readable task name used only for UI narration." }
            },
            "required": ["prompt", "repository"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_open_world(true)
            .with_requires_secrets(true)
            .with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        context_required_error("cursor_launch_agent")
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let prompt = match required_limited_str(&arguments, "prompt", MAX_PROMPT_CHARS) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let repository = match required_str(&arguments, "repository") {
            Ok(v) => v,
            Err(e) => return e,
        };
        let ref_ = match optional_str(&arguments, "ref", MAX_REF_CHARS) {
            Ok(v) => v.map(str::to_string),
            Err(e) => return e,
        };
        let model = match optional_str(&arguments, "model", MAX_MODEL_CHARS) {
            Ok(v) => v.map(str::to_string),
            Err(e) => return e,
        };
        let branch_name = match optional_str(&arguments, "branch_name", MAX_BRANCH_CHARS) {
            Ok(v) => v.map(str::to_string),
            Err(e) => return e,
        };
        let auto_create_pr = match optional_bool(&arguments, "auto_create_pr") {
            Ok(v) => v,
            Err(e) => return e,
        };
        let api_key = match get_api_key(context).await {
            Ok(v) => v,
            Err(e) => return e,
        };

        match cursor_client(api_key)
            .launch_agent(LaunchAgentRequest {
                prompt: prompt.to_string(),
                repository: repository.to_string(),
                ref_,
                model,
                auto_create_pr,
                branch_name,
            })
            .await
        {
            Ok(agent) => ToolExecutionResult::success(json!(agent)),
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }
}

pub struct CursorGetAgentTool;

#[async_trait]
impl Tool for CursorGetAgentTool {
    fn name(&self) -> &str {
        "cursor_get_agent"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Get Cursor Agent")
    }

    fn description(&self) -> &str {
        "Get status and result metadata for a Cursor Cloud Agent."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent_id": { "type": "string", "description": "Cursor agent id, e.g. bc_abc123.", "minLength": 1, "maxLength": MAX_AGENT_ID_CHARS }
            },
            "required": ["agent_id"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        context_required_error("cursor_get_agent")
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let agent_id = match required_limited_str(&arguments, "agent_id", MAX_AGENT_ID_CHARS) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let api_key = match get_api_key(context).await {
            Ok(v) => v,
            Err(e) => return e,
        };
        match cursor_client(api_key).get_agent(agent_id).await {
            Ok(agent) => ToolExecutionResult::success(json!(agent)),
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }
}

pub struct CursorListAgentsTool;

#[async_trait]
impl Tool for CursorListAgentsTool {
    fn name(&self) -> &str {
        "cursor_list_agents"
    }

    fn display_name(&self) -> Option<&str> {
        Some("List Cursor Agents")
    }

    fn description(&self) -> &str {
        "List Cursor Cloud Agents for the authenticated Cursor account."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "limit": { "type": "integer", "description": "Number of agents to return, 1-100. Default 20.", "minimum": 1, "maximum": 100 },
                "cursor": { "type": "string", "description": "Pagination cursor from a previous response." }
            },
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        context_required_error("cursor_list_agents")
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let limit = match optional_limit(&arguments) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let cursor = match optional_str(&arguments, "cursor", MAX_AGENT_ID_CHARS) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let api_key = match get_api_key(context).await {
            Ok(v) => v,
            Err(e) => return e,
        };
        match cursor_client(api_key).list_agents(limit, cursor).await {
            Ok(response) => ToolExecutionResult::success(json!(response)),
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }
}

pub struct CursorAddFollowupTool;

#[async_trait]
impl Tool for CursorAddFollowupTool {
    fn name(&self) -> &str {
        "cursor_add_followup"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Add Cursor Follow-up")
    }

    fn description(&self) -> &str {
        "Send additional instructions to a running Cursor Cloud Agent."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent_id": { "type": "string", "description": "Cursor agent id.", "minLength": 1, "maxLength": MAX_AGENT_ID_CHARS },
                "prompt": { "type": "string", "description": "Follow-up instruction.", "minLength": 1, "maxLength": MAX_PROMPT_CHARS }
            },
            "required": ["agent_id", "prompt"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        context_required_error("cursor_add_followup")
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let agent_id = match required_limited_str(&arguments, "agent_id", MAX_AGENT_ID_CHARS) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let prompt = match required_limited_str(&arguments, "prompt", MAX_PROMPT_CHARS) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let api_key = match get_api_key(context).await {
            Ok(v) => v,
            Err(e) => return e,
        };
        match cursor_client(api_key).add_followup(agent_id, prompt).await {
            Ok(response) => ToolExecutionResult::success(response),
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }
}

pub struct CursorGetConversationTool;

#[async_trait]
impl Tool for CursorGetConversationTool {
    fn name(&self) -> &str {
        "cursor_get_conversation"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Get Cursor Conversation")
    }

    fn description(&self) -> &str {
        "Retrieve the conversation transcript for a Cursor Cloud Agent."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent_id": { "type": "string", "description": "Cursor agent id.", "minLength": 1, "maxLength": MAX_AGENT_ID_CHARS }
            },
            "required": ["agent_id"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        context_required_error("cursor_get_conversation")
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let agent_id = match required_limited_str(&arguments, "agent_id", MAX_AGENT_ID_CHARS) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let api_key = match get_api_key(context).await {
            Ok(v) => v,
            Err(e) => return e,
        };
        match cursor_client(api_key).get_conversation(agent_id).await {
            Ok(response) => ToolExecutionResult::success(json!(response)),
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }
}

pub struct CursorDeleteAgentTool;

#[async_trait]
impl Tool for CursorDeleteAgentTool {
    fn name(&self) -> &str {
        "cursor_delete_agent"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Delete Cursor Agent")
    }

    fn description(&self) -> &str {
        "Permanently delete a Cursor Cloud Agent record and associated resources."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agent_id": { "type": "string", "description": "Cursor agent id.", "minLength": 1, "maxLength": MAX_AGENT_ID_CHARS }
            },
            "required": ["agent_id"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_destructive(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        context_required_error("cursor_delete_agent")
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let agent_id = match required_limited_str(&arguments, "agent_id", MAX_AGENT_ID_CHARS) {
            Ok(v) => v,
            Err(e) => return e,
        };
        let api_key = match get_api_key(context).await {
            Ok(v) => v,
            Err(e) => return e,
        };
        match cursor_client(api_key).delete_agent(agent_id).await {
            Ok(response) => ToolExecutionResult::success(response),
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }
}

pub struct CursorListModelsTool;

#[async_trait]
impl Tool for CursorListModelsTool {
    fn name(&self) -> &str {
        "cursor_list_models"
    }

    fn display_name(&self) -> Option<&str> {
        Some("List Cursor Models")
    }

    fn description(&self) -> &str {
        "List model ids recommended by Cursor for Cloud Agents."
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        context_required_error("cursor_list_models")
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let api_key = match get_api_key(context).await {
            Ok(v) => v,
            Err(e) => return e,
        };
        match cursor_client(api_key).list_models().await {
            Ok(response) => ToolExecutionResult::success(json!(response)),
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }
}

pub struct CursorListRepositoriesTool;

#[async_trait]
impl Tool for CursorListRepositoriesTool {
    fn name(&self) -> &str {
        "cursor_list_repositories"
    }

    fn display_name(&self) -> Option<&str> {
        Some("List Cursor Repositories")
    }

    fn description(&self) -> &str {
        "List GitHub repositories accessible to Cursor. This Cursor endpoint is heavily rate-limited; use sparingly."
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_open_world(true)
            .with_requires_secrets(true)
            .with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        context_required_error("cursor_list_repositories")
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let api_key = match get_api_key(context).await {
            Ok(v) => v,
            Err(e) => return e,
        };
        match cursor_client(api_key).list_repositories().await {
            Ok(response) => ToolExecutionResult::success(json!(response)),
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }
}

pub struct CursorKeyInfoTool;

#[async_trait]
impl Tool for CursorKeyInfoTool {
    fn name(&self) -> &str {
        "cursor_key_info"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Check Cursor Connection")
    }

    fn description(&self) -> &str {
        "Check Cursor API key metadata for the active connection."
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        context_required_error("cursor_key_info")
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let api_key = match get_api_key(context).await {
            Ok(v) => v,
            Err(e) => return e,
        };
        match cursor_client(api_key).api_key_info().await {
            Ok(response) => ToolExecutionResult::success(json!(response)),
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }
}

#[cfg(test)]
mod auth_tests {
    use super::*;
    use everruns_core::SessionId;

    /// Regression: ensure tool auth no longer falls back to the process-wide
    /// `CURSOR_API_KEY` env var. When no session-scoped credential is
    /// available, `get_api_key` must surface `ConnectionRequired` even if the
    /// env var is set, so the per-session credential boundary cannot be
    /// reintroduced accidentally.
    #[tokio::test]
    async fn get_api_key_does_not_fall_back_to_global_env_var() {
        // SAFETY: single-threaded test, no other code reads CURSOR_API_KEY here.
        unsafe { std::env::set_var(CURSOR_API_KEY_SECRET, "should-not-be-used") };
        let ctx = ToolContext::new(SessionId::new());
        let err = get_api_key(&ctx).await.unwrap_err();
        // SAFETY: same guarantee as set_var above; clean up before assert.
        unsafe { std::env::remove_var(CURSOR_API_KEY_SECRET) };
        match err {
            ToolExecutionResult::ConnectionRequired { provider } => {
                assert_eq!(provider, CURSOR_CONNECTION_PROVIDER);
            }
            other => panic!("expected ConnectionRequired, got {other:?}"),
        }
    }
}
