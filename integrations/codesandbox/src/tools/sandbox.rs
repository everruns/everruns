//! Sandbox lifecycle tools: create, list, manage.

use crate::client::CodeSandboxClient;
use crate::state::*;
use crate::tools::exec::poll_exec_completion;
use crate::types::*;

use async_trait::async_trait;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;
use serde_json::{Value, json};
use tracing::{debug, warn};

// ----------------------------------------------------------------------------
// CsbCreateSandboxTool
// ----------------------------------------------------------------------------

pub struct CsbCreateSandboxTool;

#[async_trait]
impl Tool for CsbCreateSandboxTool {
    fn name(&self) -> &str {
        "csb_create_sandbox"
    }

    fn description(&self) -> &str {
        "Create a new CodeSandbox cloud VM. Optionally upload files from session storage."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Sandbox title (optional)"
                },
                "upload_files": {
                    "type": "array",
                    "description": "Files to upload from session storage (optional)",
                    "items": {
                        "type": "object",
                        "properties": {
                            "session_path": { "type": "string", "description": "Source path in session storage" },
                            "sandbox_path": { "type": "string", "description": "Destination path in sandbox" }
                        },
                        "required": ["session_path", "sandbox_path"]
                    }
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "csb_create_sandbox requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };

        let client = CodeSandboxClient::new(api_key);

        // Build create request
        let title = arguments
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Everruns Sandbox");
        let create_body = json!({
            "title": title,
            "privacy": 2,
            "runtime": "vm",
            "settings": { "use_pint": true },
            "hibernationTimeoutSeconds": HIBERNATE_TIMEOUT_SECS,
        });

        // Create sandbox
        debug!("Creating CodeSandbox: {title}");
        let sandbox_info = match client.create_sandbox(create_body).await {
            Ok(info) => info,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        let sandbox_id = &sandbox_info.id;

        // Start VM
        debug!("Starting VM for sandbox: {sandbox_id}");
        let vm_info = match client.start_vm(sandbox_id, None).await {
            Ok(info) => info,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        // Use /sandbox as consistent working directory across all sandbox providers
        let workspace_path = "/sandbox".to_string();

        // Create preview token (required for Pint API port proxy auth)
        debug!("Creating preview token for sandbox: {sandbox_id}");
        let preview_token_resp = match client.create_preview_token(sandbox_id).await {
            Ok(resp) => resp,
            Err(e) => {
                return ToolExecutionResult::tool_error(format!(
                    "Failed to create preview token: {e}"
                ));
            }
        };
        let preview_token = preview_token_resp.token.token;

        // Pint API is accessed via port forwarding: https://{sandbox_id}-57468.csb.app
        let pint_url = format!("https://{sandbox_id}-57468.csb.app");

        // Wait for Pint API to be ready (VM service may still be booting)
        debug!("Waiting for Pint API to become ready...");
        if let Err(e) = client
            .wait_for_pint_ready(&pint_url, &vm_info.pitcher_token, &preview_token)
            .await
        {
            warn!("Pint API readiness check failed: {e}");
            // Continue anyway — the sandbox was created, agent can retry later
        }

        // Save state (needed before mkdir so exec can look up the sandbox)
        let state = SandboxState {
            sandbox_id: sandbox_id.clone(),
            pint_url: pint_url.clone(),
            pitcher_token: vm_info.pitcher_token.clone(),
            preview_token: preview_token.clone(),
            workspace_path: workspace_path.clone(),
            started_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Err(e) = save_sandbox_state(context, &state).await {
            return e;
        }

        // Ensure /sandbox directory exists
        if let Ok(exec_info) = client
            .exec_create(
                &state,
                "bash",
                vec!["-c".to_string(), "mkdir -p /sandbox".to_string()],
            )
            .await
        {
            let _ = poll_exec_completion(&client, &state, &exec_info.id).await;
        }

        // Optionally upload files
        let mut uploaded_count = 0;
        if let Some(files) = arguments.get("upload_files").and_then(|v| v.as_array()) {
            let file_store = match context.file_store.as_ref() {
                Some(fs) => fs,
                None => {
                    return ToolExecutionResult::tool_error(
                        "File store not available for file upload",
                    );
                }
            };

            for file_spec in files {
                let session_path = match file_spec.get("session_path").and_then(|v| v.as_str()) {
                    Some(p) => p,
                    None => continue,
                };
                let sandbox_path = match file_spec.get("sandbox_path").and_then(|v| v.as_str()) {
                    Some(p) => p,
                    None => continue,
                };

                // Read from session storage
                match file_store.read_file(context.session_id, session_path).await {
                    Ok(Some(file)) => {
                        // Write to sandbox
                        let content = file.content.unwrap_or_default();
                        if let Err(e) = client.file_write(&state, sandbox_path, &content).await {
                            warn!("Failed to upload {session_path} to sandbox: {e}");
                        } else {
                            uploaded_count += 1;
                        }
                    }
                    Ok(None) => {
                        warn!("File not found in session storage: {session_path}");
                    }
                    Err(e) => {
                        warn!("Failed to read {session_path}: {e}");
                    }
                }
            }
        }

        ToolExecutionResult::success(json!({
            "sandbox_id": sandbox_id,
            "status": "running",
            "workspace_path": workspace_path,
            "files_uploaded": uploaded_count
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ----------------------------------------------------------------------------
// CsbListSandboxesTool
// ----------------------------------------------------------------------------

pub struct CsbListSandboxesTool;

#[async_trait]
impl Tool for CsbListSandboxesTool {
    fn name(&self) -> &str {
        "csb_list_sandboxes"
    }

    fn description(&self) -> &str {
        "List all CodeSandbox VMs created in this session."
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
            "csb_list_sandboxes requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let states = match list_sandbox_states(context).await {
            Ok(s) => s,
            Err(e) => return e,
        };

        let sandboxes: Vec<Value> = states
            .iter()
            .map(|s| {
                json!({
                    "sandbox_id": s.sandbox_id,
                    "started_at": s.started_at,
                    "workspace_path": s.workspace_path
                })
            })
            .collect();

        ToolExecutionResult::success(json!({
            "sandboxes": sandboxes,
            "count": sandboxes.len()
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ----------------------------------------------------------------------------
// CsbManageSandboxTool
// ----------------------------------------------------------------------------

pub struct CsbManageSandboxTool;

#[async_trait]
impl Tool for CsbManageSandboxTool {
    fn name(&self) -> &str {
        "csb_manage_sandbox"
    }

    fn description(&self) -> &str {
        "Manage sandbox lifecycle: shutdown, hibernate, or delete a CodeSandbox VM."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {
                    "type": "string",
                    "description": "Sandbox ID"
                },
                "action": {
                    "type": "string",
                    "enum": ["shutdown", "hibernate", "delete"],
                    "description": "Action to perform: shutdown (stop and delete VM), hibernate (save state, keeps on dashboard), delete (same as shutdown)"
                }
            },
            "required": ["sandbox_id", "action"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "csb_manage_sandbox requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = match required_str(&arguments, "sandbox_id") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let action = match required_str(&arguments, "action") {
            Ok(s) => s,
            Err(e) => return e,
        };

        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        // Verify sandbox exists in state
        if let Err(e) = get_sandbox_state(context, sandbox_id).await {
            return e;
        }

        let client = CodeSandboxClient::new(api_key);

        let result = match action {
            "shutdown" | "delete" => {
                // Both shutdown and delete fully remove the sandbox.
                // Plain shutdown leaves sandboxes lingering on the dashboard,
                // so we always delete to clean up.
                let r = client.delete_vm(sandbox_id).await;
                if r.is_ok() {
                    let _ = delete_sandbox_state(context, sandbox_id).await;
                }
                r
            }
            "hibernate" => client.hibernate_vm(sandbox_id).await,
            _ => {
                return ToolExecutionResult::tool_error(format!(
                    "Invalid action: '{action}'. Must be one of: shutdown, hibernate, delete"
                ));
            }
        };

        match result {
            Ok(()) => ToolExecutionResult::success(json!({
                "sandbox_id": sandbox_id,
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

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::tools::Tool;

    #[test]
    fn test_create_sandbox_schema_no_required() {
        let tool = CsbCreateSandboxTool;
        let schema = tool.parameters_schema();
        assert!(schema.get("required").is_none());
    }

    #[test]
    fn test_manage_sandbox_schema_has_enum() {
        let tool = CsbManageSandboxTool;
        let schema = tool.parameters_schema();
        let action_enum = &schema["properties"]["action"]["enum"];
        let values: Vec<&str> = action_enum
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(values.contains(&"shutdown"));
        assert!(values.contains(&"hibernate"));
        assert!(values.contains(&"delete"));
    }

    #[test]
    fn test_list_sandboxes_schema_empty_properties() {
        let tool = CsbListSandboxesTool;
        let schema = tool.parameters_schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.is_empty());
    }

    #[tokio::test]
    async fn test_create_sandbox_without_context() {
        let tool = CsbCreateSandboxTool;
        let result = tool.execute(json!({})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("requires context")),
            _ => panic!("Expected tool error"),
        }
    }

    #[tokio::test]
    async fn test_list_sandboxes_without_context() {
        let tool = CsbListSandboxesTool;
        let result = tool.execute(json!({})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("requires context")),
            _ => panic!("Expected tool error"),
        }
    }

    #[tokio::test]
    async fn test_manage_sandbox_without_context() {
        let tool = CsbManageSandboxTool;
        let result = tool
            .execute(json!({"sandbox_id": "test", "action": "shutdown"}))
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("requires context")),
            _ => panic!("Expected tool error"),
        }
    }

    #[test]
    fn test_all_sandbox_tools_require_context() {
        assert!(CsbCreateSandboxTool.requires_context());
        assert!(CsbListSandboxesTool.requires_context());
        assert!(CsbManageSandboxTool.requires_context());
    }
}
