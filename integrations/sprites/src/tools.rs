//! Tool implementations for Sprites operations.

use everruns_core::tool_types::ToolHints;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;

use async_trait::async_trait;
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::EXEC_TIMEOUT_MS;
use crate::client::SpritesClient;
use crate::state::{
    SpriteState, delete_sprite_state, get_api_token, get_sprite_state, list_sprite_states,
    release_sprite_lease, required_str, save_sprite_state, touch_sprite_lease,
    validate_sprite_name,
};

/// Parse an optional integer resource parameter, validating type and range.
fn parse_resource_param(
    arguments: &Value,
    name: &str,
    min: u64,
    max: u64,
) -> Result<Option<u64>, String> {
    let Some(val) = arguments.get(name) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let n = val
        .as_u64()
        .ok_or_else(|| format!("Invalid '{name}': must be a positive integer"))?;
    if n < min || n > max {
        return Err(format!("Invalid '{name}': must be between {min} and {max}"));
    }
    Ok(Some(n))
}

// ============================================================================
// SpritesCreateSpriteTool
// ============================================================================

pub struct SpritesCreateSpriteTool;

#[async_trait]
impl Tool for SpritesCreateSpriteTool {
    fn name(&self) -> &str {
        "sprites_create_sprite"
    }

    fn description(&self) -> &str {
        "Create a new Sprites microVM. Returns a persistent Linux environment with full filesystem."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sprite_name": {
                    "type": "string",
                    "description": "Unique name for the sprite (lowercase, hyphens allowed)"
                },
                "cpus": {
                    "type": "integer",
                    "description": "Number of CPUs (1-8, default: 1)",
                    "minimum": 1,
                    "maximum": 8,
                    "default": 1
                },
                "memory_mb": {
                    "type": "integer",
                    "description": "Memory in MB (256-16384, default: 512)",
                    "minimum": 256,
                    "maximum": 16384,
                    "default": 512
                }
            },
            "required": ["sprite_name"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "sprites_create_sprite requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let api_token = match get_api_token(context).await {
            Ok(t) => t,
            Err(e) => return e,
        };

        let sprite_name = match required_str(&arguments, "sprite_name") {
            Ok(s) => s,
            Err(e) => return e,
        };
        if let Err(e) = validate_sprite_name(sprite_name) {
            return e;
        }

        let client = SpritesClient::new(api_token);

        // Parse resource constraints
        let cpus = match parse_resource_param(&arguments, "cpus", 1, 8) {
            Ok(v) => v,
            Err(e) => return ToolExecutionResult::tool_error(&e),
        };
        let memory_mb = match parse_resource_param(&arguments, "memory_mb", 256, 16384) {
            Ok(v) => v,
            Err(e) => return ToolExecutionResult::tool_error(&e),
        };

        // Build create request with metadata labels
        let mut create_body = json!({
            "metadata": {
                "everruns": "true",
                "everruns.session_id": context.session_id.to_string(),
            }
        });
        if let Some(session_store) = &context.session_store
            && let Ok(Some(session)) = session_store.get_session(context.session_id).await
        {
            create_body["metadata"]["everruns.harness_id"] = json!(session.harness_id.to_string());
            create_body["metadata"]["everruns.org_id"] = json!(&session.organization_id);
            if let Some(agent_id) = &session.agent_id {
                create_body["metadata"]["everruns.agent_id"] = json!(agent_id.to_string());
            }
        }

        if let Some(cpus) = cpus {
            create_body["guest"] = json!({"cpus": cpus});
        }
        if let Some(mem) = memory_mb {
            if let Some(guest) = create_body.get_mut("guest") {
                guest["memory_mb"] = json!(mem);
            } else {
                create_body["guest"] = json!({"memory_mb": mem});
            }
        }

        debug!("Creating sprite: {sprite_name}");
        let sprite_info = match client.create_sprite(sprite_name, create_body).await {
            Ok(info) => info,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        let workspace_path = crate::SPRITES_WORKSPACE_PATH.to_string();

        // Save state
        let state = SpriteState {
            sprite_name: sprite_name.to_string(),
            workspace_path: workspace_path.clone(),
            started_at: chrono::Utc::now().to_rfc3339(),
            service_url: sprite_info.url.clone(),
        };
        if let Err(e) = save_sprite_state(context, &state).await {
            return e;
        }
        if let Err(e) = touch_sprite_lease(context, &state, Some(sprite_name.to_string())).await {
            return e;
        }

        let mut result = json!({
            "sprite_name": sprite_name,
            "status": sprite_info.status,
            "workspace_path": workspace_path,
        });
        if let Some(url) = &sprite_info.url {
            result["service_url"] = json!(url);
        }

        ToolExecutionResult::success(result)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// SpritesExecTool
// ============================================================================

pub struct SpritesExecTool;

#[async_trait]
impl Tool for SpritesExecTool {
    fn name(&self) -> &str {
        "sprites_exec"
    }

    fn description(&self) -> &str {
        "Execute a shell command in a Sprites microVM. The sprite wakes instantly if hibernating."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sprite_name": {
                    "type": "string",
                    "description": "Sprite name to execute in"
                },
                "command": {
                    "type": "string",
                    "description": "Shell command to execute (e.g., 'python app.py')"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (optional, default: 120000)"
                },
                "output": everruns_core::tool_output_sanitizer::output_verbosity_schema()
            },
            "required": ["sprite_name", "command"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_open_world(true)
            .with_requires_secrets(true)
            .with_long_running(true)
            .with_persist_output(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "sprites_exec requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sprite_name = match required_str(&arguments, "sprite_name") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let command = match required_str(&arguments, "command") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let output_mode = arguments
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or("concise");
        let timeout = arguments
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(EXEC_TIMEOUT_MS)
            .min(600_000); // Cap at 10 minutes

        let api_token = match get_api_token(context).await {
            Ok(t) => t,
            Err(e) => return e,
        };
        let state = match get_sprite_state(context, sprite_name).await {
            Ok(state) => state,
            Err(e) => return e,
        };

        let client = SpritesClient::new(api_token);

        debug!("Executing in sprite {sprite_name}: {command}");
        match client.exec(sprite_name, command, Some(timeout)).await {
            Ok(result) => {
                if let Err(e) = touch_sprite_lease(context, &state, None).await {
                    return e;
                }
                use everruns_core::tool_output_sanitizer::{
                    clean_exec_output, output_verbosity_budget, priority_aware_truncate,
                };
                let clean_stdout = clean_exec_output(&result.stdout);
                let clean_stderr = clean_exec_output(&result.stderr);
                let (stdout, stderr) = if let Some(budget) = output_verbosity_budget(output_mode) {
                    (
                        priority_aware_truncate(&clean_stdout, budget),
                        priority_aware_truncate(&clean_stderr, budget.min(4096)),
                    )
                } else {
                    (clean_stdout.clone(), clean_stderr.clone())
                };
                let mut raw = clean_stdout;
                if !clean_stderr.is_empty() {
                    raw.push_str("\n--- stderr ---\n");
                    raw.push_str(&clean_stderr);
                }
                ToolExecutionResult::success_with_raw_output(
                    json!({
                        "exit_code": result.exit_code,
                        "stdout": stdout,
                        "stderr": stderr,
                    }),
                    raw,
                )
            }
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// SpritesReadFileTool
// ============================================================================

pub struct SpritesReadFileTool;

#[async_trait]
impl Tool for SpritesReadFileTool {
    fn name(&self) -> &str {
        "sprites_read_file"
    }

    fn description(&self) -> &str {
        "Read a file from a remote Sprites microVM filesystem (NOT the session /workspace). Requires sprite_name."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sprite_name": {
                    "type": "string",
                    "description": "Sprite name"
                },
                "path": {
                    "type": "string",
                    "description": "Path to file (e.g., '/home/user/main.py')"
                }
            },
            "required": ["sprite_name", "path"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "sprites_read_file requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sprite_name = match required_str(&arguments, "sprite_name") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let path = match required_str(&arguments, "path") {
            Ok(s) => s,
            Err(e) => return e,
        };

        let api_token = match get_api_token(context).await {
            Ok(t) => t,
            Err(e) => return e,
        };
        let state = match get_sprite_state(context, sprite_name).await {
            Ok(state) => state,
            Err(e) => return e,
        };

        let client = SpritesClient::new(api_token);

        match client.read_file(sprite_name, path).await {
            Ok(bytes) => {
                if let Err(e) = touch_sprite_lease(context, &state, None).await {
                    return e;
                }
                let content = String::from_utf8_lossy(&bytes).to_string();
                ToolExecutionResult::success(json!({
                    "path": path,
                    "content": content
                }))
            }
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// SpritesWriteFileTool
// ============================================================================

pub struct SpritesWriteFileTool;

#[async_trait]
impl Tool for SpritesWriteFileTool {
    fn name(&self) -> &str {
        "sprites_write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file in a Sprites microVM filesystem."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sprite_name": {
                    "type": "string",
                    "description": "Sprite name"
                },
                "path": {
                    "type": "string",
                    "description": "Path for file (e.g., '/home/user/main.py')"
                },
                "content": {
                    "type": "string",
                    "description": "File content to write"
                }
            },
            "required": ["sprite_name", "path", "content"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "sprites_write_file requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sprite_name = match required_str(&arguments, "sprite_name") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let path = match required_str(&arguments, "path") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let content = match required_str(&arguments, "content") {
            Ok(s) => s,
            Err(e) => return e,
        };

        let api_token = match get_api_token(context).await {
            Ok(t) => t,
            Err(e) => return e,
        };
        let state = match get_sprite_state(context, sprite_name).await {
            Ok(state) => state,
            Err(e) => return e,
        };

        let client = SpritesClient::new(api_token);

        match client
            .write_file(sprite_name, path, content.as_bytes())
            .await
        {
            Ok(()) => {
                if let Err(e) = touch_sprite_lease(context, &state, None).await {
                    return e;
                }
                ToolExecutionResult::success(json!({
                    "path": path,
                    "success": true
                }))
            }
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// SpritesListSpritesTool
// ============================================================================

pub struct SpritesListSpritesTool;

#[async_trait]
impl Tool for SpritesListSpritesTool {
    fn name(&self) -> &str {
        "sprites_list_sprites"
    }

    fn description(&self) -> &str {
        "List all Sprites microVMs created in this session."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "sprites_list_sprites requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let states = match list_sprite_states(context).await {
            Ok(s) => s,
            Err(e) => return e,
        };

        let sprites: Vec<Value> = states
            .iter()
            .map(|s| {
                let mut entry = json!({
                    "sprite_name": s.sprite_name,
                    "started_at": s.started_at,
                    "workspace_path": s.workspace_path,
                });
                if let Some(url) = &s.service_url {
                    entry["service_url"] = json!(url);
                }
                entry
            })
            .collect();

        ToolExecutionResult::success(json!({
            "sprites": sprites,
            "count": sprites.len()
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// SpritesManageSpriteTool
// ============================================================================

pub struct SpritesManageSpriteTool;

#[async_trait]
impl Tool for SpritesManageSpriteTool {
    fn name(&self) -> &str {
        "sprites_manage_sprite"
    }

    fn description(&self) -> &str {
        "Manage sprite lifecycle: delete a Sprites microVM."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sprite_name": {
                    "type": "string",
                    "description": "Sprite name"
                },
                "action": {
                    "type": "string",
                    "enum": ["delete"],
                    "description": "Action to perform: delete (permanently remove sprite and all data)"
                }
            },
            "required": ["sprite_name", "action"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "sprites_manage_sprite requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sprite_name = match required_str(&arguments, "sprite_name") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let action = match required_str(&arguments, "action") {
            Ok(s) => s,
            Err(e) => return e,
        };

        let api_token = match get_api_token(context).await {
            Ok(t) => t,
            Err(e) => return e,
        };
        let _state = match get_sprite_state(context, sprite_name).await {
            Ok(state) => state,
            Err(e) => return e,
        };

        let client = SpritesClient::new(api_token);

        let result = match action {
            "delete" => {
                let r = client.delete_sprite(sprite_name).await;
                if r.is_ok() {
                    let _ = delete_sprite_state(context, sprite_name).await;
                    let _ = release_sprite_lease(context, sprite_name).await;
                }
                r
            }
            _ => {
                return ToolExecutionResult::tool_error(format!(
                    "Invalid action: '{action}'. Must be one of: delete"
                ));
            }
        };

        match result {
            Ok(()) => ToolExecutionResult::success(json!({
                "sprite_name": sprite_name,
                "action": action,
                "success": true
            })),
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// SpritesCheckpointTool
// ============================================================================

pub struct SpritesCheckpointTool;

#[async_trait]
impl Tool for SpritesCheckpointTool {
    fn name(&self) -> &str {
        "sprites_checkpoint"
    }

    fn description(&self) -> &str {
        "Create a checkpoint (snapshot) of a Sprites microVM. Captures the full filesystem \
         state in ~300ms without interrupting the sprite. Use before risky operations."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sprite_name": {
                    "type": "string",
                    "description": "Sprite name to checkpoint"
                }
            },
            "required": ["sprite_name"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "sprites_checkpoint requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sprite_name = match required_str(&arguments, "sprite_name") {
            Ok(s) => s,
            Err(e) => return e,
        };

        let api_token = match get_api_token(context).await {
            Ok(t) => t,
            Err(e) => return e,
        };
        let state = match get_sprite_state(context, sprite_name).await {
            Ok(state) => state,
            Err(e) => return e,
        };

        let client = SpritesClient::new(api_token);

        match client.create_checkpoint(sprite_name).await {
            Ok(checkpoint) => {
                if let Err(e) = touch_sprite_lease(context, &state, None).await {
                    return e;
                }
                let mut result = json!({
                    "sprite_name": sprite_name,
                    "checkpoint_id": checkpoint.id,
                    "success": true
                });
                if let Some(created_at) = &checkpoint.created_at {
                    result["created_at"] = json!(created_at);
                }
                ToolExecutionResult::success(result)
            }
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// SpritesRestoreCheckpointTool
// ============================================================================

pub struct SpritesRestoreCheckpointTool;

#[async_trait]
impl Tool for SpritesRestoreCheckpointTool {
    fn name(&self) -> &str {
        "sprites_restore_checkpoint"
    }

    fn description(&self) -> &str {
        "Restore a Sprites microVM to a previously created checkpoint. Rolls back the \
         filesystem to the snapshot state."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sprite_name": {
                    "type": "string",
                    "description": "Sprite name"
                },
                "checkpoint_id": {
                    "type": "string",
                    "description": "Checkpoint ID to restore (from sprites_checkpoint output)"
                }
            },
            "required": ["sprite_name", "checkpoint_id"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "sprites_restore_checkpoint requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sprite_name = match required_str(&arguments, "sprite_name") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let checkpoint_id = match required_str(&arguments, "checkpoint_id") {
            Ok(s) => s,
            Err(e) => return e,
        };

        let api_token = match get_api_token(context).await {
            Ok(t) => t,
            Err(e) => return e,
        };
        let state = match get_sprite_state(context, sprite_name).await {
            Ok(state) => state,
            Err(e) => return e,
        };

        let client = SpritesClient::new(api_token);

        match client.restore_checkpoint(sprite_name, checkpoint_id).await {
            Ok(()) => {
                if let Err(e) = touch_sprite_lease(context, &state, None).await {
                    return e;
                }
                ToolExecutionResult::success(json!({
                    "sprite_name": sprite_name,
                    "checkpoint_id": checkpoint_id,
                    "restored": true
                }))
            }
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// SpritesServiceUrlTool
// ============================================================================

pub struct SpritesServiceUrlTool;

#[async_trait]
impl Tool for SpritesServiceUrlTool {
    fn name(&self) -> &str {
        "sprites_service_url"
    }

    fn description(&self) -> &str {
        "Get the public HTTP URL for a Sprites microVM. Services listening on port 8080 \
         inside the sprite are accessible at this URL."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sprite_name": {
                    "type": "string",
                    "description": "Sprite name"
                }
            },
            "required": ["sprite_name"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "sprites_service_url requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sprite_name = match required_str(&arguments, "sprite_name") {
            Ok(s) => s,
            Err(e) => return e,
        };

        let api_token = match get_api_token(context).await {
            Ok(t) => t,
            Err(e) => return e,
        };
        let state = match get_sprite_state(context, sprite_name).await {
            Ok(state) => state,
            Err(e) => return e,
        };

        let client = SpritesClient::new(api_token);

        // Fetch fresh sprite info and services to get the URL
        let sprite_info = match client.get_sprite(sprite_name).await {
            Ok(info) => info,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        let services = match client.list_services(sprite_name).await {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to list services: {e}");
                vec![]
            }
        };

        // Persist URL back to state so list_sprites shows it
        let mut state = state;
        if sprite_info.url.is_some() && state.service_url != sprite_info.url {
            state.service_url.clone_from(&sprite_info.url);
            if let Err(e) = save_sprite_state(context, &state).await {
                return e;
            }
        }

        if let Err(e) = touch_sprite_lease(context, &state, None).await {
            return e;
        }

        let mut result = json!({
            "sprite_name": sprite_name,
            "status": sprite_info.status,
        });

        if let Some(url) = &sprite_info.url {
            result["url"] = json!(url);
        }
        if !services.is_empty() {
            result["services"] = json!(services);
        }

        ToolExecutionResult::success(result)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::capabilities::Capability;

    fn get_tool(name: &str) -> Box<dyn Tool> {
        let cap = crate::SpritesCapability;
        cap.tools()
            .into_iter()
            .find(|t| t.name() == name)
            .unwrap_or_else(|| panic!("Tool {name} not found"))
    }

    #[test]
    fn test_all_tools_have_schema() {
        let cap = crate::SpritesCapability;
        for tool in cap.tools() {
            let schema = tool.parameters_schema();
            assert!(
                schema.get("type").is_some(),
                "Tool {} missing type in schema",
                tool.name()
            );
        }
    }

    #[tokio::test]
    async fn test_exec_tool_requires_context() {
        let tool = get_tool("sprites_exec");
        let result = tool
            .execute(json!({"sprite_name": "test", "command": "ls"}))
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"), "Got: {msg}");
            }
            other => panic!("Expected ToolError, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_create_tool_requires_context() {
        let tool = get_tool("sprites_create_sprite");
        let result = tool.execute(json!({"sprite_name": "test"})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"), "Got: {msg}");
            }
            other => panic!("Expected ToolError, got: {other:?}"),
        }
    }

    #[test]
    fn test_parse_resource_valid() {
        let args = json!({"cpus": 4});
        assert_eq!(parse_resource_param(&args, "cpus", 1, 8).unwrap(), Some(4));
    }

    #[test]
    fn test_parse_resource_missing() {
        let args = json!({});
        assert_eq!(parse_resource_param(&args, "cpus", 1, 8).unwrap(), None);
    }

    #[test]
    fn test_parse_resource_null() {
        let args = json!({"cpus": null});
        assert_eq!(parse_resource_param(&args, "cpus", 1, 8).unwrap(), None);
    }

    #[test]
    fn test_parse_resource_out_of_range() {
        let args = json!({"cpus": 100});
        assert!(parse_resource_param(&args, "cpus", 1, 8).is_err());
    }

    #[test]
    fn test_parse_resource_string_type() {
        let args = json!({"cpus": "big"});
        assert!(parse_resource_param(&args, "cpus", 1, 8).is_err());
    }
}
