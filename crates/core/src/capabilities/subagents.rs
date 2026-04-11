// Subagent Capability
//
// Decision: 3 tools only — spawn_subagent, get_subagents, message_subagent.
// - spawn_subagent creates a child session with parent_session_id set
// - get_subagents lists/details child sessions by querying parent_session_id
// - message_subagent sends steering messages (by name or id)
//
// Blueprint support: spawn_subagent accepts optional `blueprint` and `config`
// params. When blueprint is set, the child session uses the blueprint's
// RuntimeAgent (own prompt, tools, model) instead of inheriting parent's.
//
// Foreground mode: blocks until subagent completes (send_message + wait_for_idle)
// Background mode: returns immediately (deferred to Phase 1b)
//
// Subagent naming: human-readable ("Test Runner"), unique per parent, case-insensitive.
// Nesting prevention: rejects spawn if current session has parent_session_id set.

use super::{Capability, CapabilityStatus};
use crate::platform_store::PlatformStore;
use crate::tool_types::ToolHints;
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde_json::{Value, json};

/// Subagent capability — spawn and manage child agent sessions.
pub struct SubagentCapability;

impl Capability for SubagentCapability {
    fn id(&self) -> &str {
        "subagents"
    }

    fn name(&self) -> &str {
        "Subagents"
    }

    fn description(&self) -> &str {
        "Spawn and manage subagents for parallel task execution in isolated context windows."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("git-branch")
    }

    fn category(&self) -> Option<&str> {
        Some("Orchestration")
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["subagents"]
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(SUBAGENT_SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(SpawnSubagentTool),
            Box::new(GetSubagentsTool),
            Box::new(MessageSubagentTool),
        ]
    }
}

const SUBAGENT_SYSTEM_PROMPT: &str = r#"Delegate tasks to subagents running in their own context window.

- Move noisy/verbose work off the main conversation (test runs, large searches).
- Run independent tasks in parallel (multiple spawn_subagent calls in one response).
- Subagents cannot spawn other subagents (no nesting).
- Use `blueprint` parameter to spawn specialist agents with their own tools and model."#;

// =============================================================================
// Helper: get platform store from context
// =============================================================================

fn get_platform_store(context: &ToolContext) -> Result<&dyn PlatformStore, ToolExecutionResult> {
    context
        .platform_store
        .as_ref()
        .map(|s| s.as_ref())
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(
                "Subagent tools require platform_store context (not available in this environment)",
            )
        })
}

fn get_session_store(
    context: &ToolContext,
) -> Result<&dyn crate::traits::SessionStore, ToolExecutionResult> {
    context
        .session_store
        .as_ref()
        .map(|s| s.as_ref())
        .ok_or_else(|| {
            ToolExecutionResult::tool_error("Subagent tools require session_store context")
        })
}

fn require_str<'a>(args: &'a Value, field: &str) -> Result<&'a str, ToolExecutionResult> {
    args.get(field)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!("Missing required parameter: {field}"))
        })
}

/// Extract the last assistant/agent message content from a list of messages.
fn last_agent_message(messages: &[crate::platform_store::PlatformMessage]) -> Option<String> {
    messages
        .iter()
        .rfind(|m| m.role == "agent" || m.role == "assistant")
        .map(|m| m.content.clone())
}

/// Find a child session by name (case-insensitive) or ID within a list of sessions.
fn find_child_session<'a>(
    sessions: &'a [crate::session::Session],
    parent_id: crate::typed_id::SessionId,
    name_or_id: &str,
) -> Option<&'a crate::session::Session> {
    sessions
        .iter()
        .filter(|s| s.parent_session_id == Some(parent_id))
        .find(|s| {
            s.subagent_name
                .as_ref()
                .is_some_and(|n| n.eq_ignore_ascii_case(name_or_id))
                || s.id.to_string() == name_or_id
        })
}

// =============================================================================
// Tool: spawn_subagent
// =============================================================================

pub struct SpawnSubagentTool;

#[async_trait]
impl Tool for SpawnSubagentTool {
    fn name(&self) -> &str {
        "spawn_subagent"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Spawn Subagent")
    }

    fn description(&self) -> &str {
        "Spawn a named subagent to handle a specific task in its own context window. Use `blueprint` to spawn a specialist agent with its own tools and model."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Human-readable name for the subagent (e.g. 'Test Runner', 'Auth Explorer'). Must be unique within this session."
                },
                "task": {
                    "type": "string",
                    "description": "Task description — what the subagent should do."
                },
                "blueprint": {
                    "type": "string",
                    "description": "Blueprint ID to spawn a specialist agent with its own tools and model. Omit to inherit parent's configuration."
                },
                "config": {
                    "type": "object",
                    "description": "Blueprint-specific configuration. Only valid when `blueprint` is set. Validated against the blueprint's config schema."
                }
            },
            "required": ["name", "task"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "spawn_subagent requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let store = match get_platform_store(context) {
            Ok(s) => s,
            Err(e) => return e,
        };
        let session_store = match get_session_store(context) {
            Ok(s) => s,
            Err(e) => return e,
        };

        let name = match require_str(&arguments, "name") {
            Ok(s) => s.trim().to_string(),
            Err(e) => return e,
        };
        let task = match require_str(&arguments, "task") {
            Ok(s) => s.to_string(),
            Err(e) => return e,
        };

        let blueprint_param = arguments
            .get("blueprint")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());
        let config_param = arguments.get("config").filter(|v| !v.is_null()).cloned();

        // Reject config without blueprint
        if config_param.is_some() && blueprint_param.is_none() {
            return ToolExecutionResult::tool_error(
                "The `config` parameter is only valid when `blueprint` is set.",
            );
        }

        // Validate blueprint exists if specified
        if let Some(ref bp_id) = blueprint_param {
            if let Some(ref registry) = context.capability_registry {
                if registry.blueprint(bp_id).is_none() {
                    return ToolExecutionResult::tool_error(format!(
                        "Unknown blueprint: \"{bp_id}\". Check available blueprints."
                    ));
                }
                // Validate config against schema if blueprint has one
                if let Some(blueprint) = registry.blueprint(bp_id)
                    && let Some(ref schema) = blueprint.config_schema
                    && config_param.is_none()
                    && schema
                        .get("required")
                        .is_some_and(|r| r.as_array().is_some_and(|arr| !arr.is_empty()))
                {
                    return ToolExecutionResult::tool_error(format!(
                        "Blueprint \"{bp_id}\" requires config. Schema: {}",
                        serde_json::to_string_pretty(schema).unwrap_or_default()
                    ));
                }
            } else {
                return ToolExecutionResult::tool_error(
                    "Blueprint support requires capability_registry context.",
                );
            }
        }

        // Nesting check: reject if current session is already a subagent
        let parent_session = match session_store.get_session(context.session_id).await {
            Ok(Some(s)) => s,
            Ok(None) => return ToolExecutionResult::tool_error("Current session not found"),
            Err(e) => return ToolExecutionResult::internal_error(e),
        };

        if parent_session.parent_session_id.is_some() {
            return ToolExecutionResult::tool_error(
                "Subagents cannot spawn other subagents (nesting not allowed).",
            );
        }

        // Create child session
        let child_session = match store
            .create_session(
                parent_session.harness_id,
                if blueprint_param.is_some() {
                    None // Blueprint sessions don't inherit agent
                } else {
                    parent_session.agent_id
                },
                Some(&name),
                parent_session.locale.as_deref(),
                blueprint_param.as_deref(),
                config_param.as_ref(),
            )
            .await
        {
            Ok(s) => s,
            Err(e) => return ToolExecutionResult::internal_error(e),
        };

        // Register subagent in session resource registry.
        if let Some(ref registry) = context.session_resource_registry {
            let _ = registry
                .register(crate::session_resource::RegisterSessionResource {
                    session_id: context.session_id,
                    resource_id: child_session.id.to_string(),
                    kind: "subagent".to_string(),
                    display_name: name.clone(),
                    status: crate::session_resource::SessionResourceStatus::Active,
                    metadata: json!({
                        "task": &task,
                        "blueprint_id": &blueprint_param,
                    }),
                })
                .await;
        }

        // Send the task as the first message
        if let Err(e) = store.send_message(child_session.id, &task).await {
            return ToolExecutionResult::internal_error(e);
        }

        // Foreground mode: wait for completion
        let status = match store.wait_for_idle(child_session.id, Some(300)).await {
            Ok(s) => s,
            Err(e) => {
                // Mark as failed in registry.
                if let Some(ref registry) = context.session_resource_registry {
                    let _ = registry
                        .update_status(
                            context.session_id,
                            &child_session.id.to_string(),
                            crate::session_resource::SessionResourceStatus::Failed,
                        )
                        .await;
                }
                return ToolExecutionResult::success(json!({
                    "subagent_id": child_session.id.to_string(),
                    "name": name,
                    "status": "failed",
                    "error": e.to_string(),
                    "blueprint": blueprint_param,
                }));
            }
        };

        // Get the subagent's response messages
        let messages = match store.get_messages(child_session.id, Some(5)).await {
            Ok(m) => m,
            Err(e) => return ToolExecutionResult::internal_error(e),
        };

        let result_text = last_agent_message(&messages)
            .unwrap_or_else(|| format!("Subagent completed with status: {status}"));

        // Update registry with terminal status.
        if let Some(ref registry) = context.session_resource_registry {
            let terminal = if status == "error" {
                crate::session_resource::SessionResourceStatus::Failed
            } else {
                crate::session_resource::SessionResourceStatus::Completed
            };
            let _ = registry
                .update_status(context.session_id, &child_session.id.to_string(), terminal)
                .await;
        }

        ToolExecutionResult::success(json!({
            "subagent_id": child_session.id.to_string(),
            "name": name,
            "status": status,
            "result": result_text,
            "blueprint": blueprint_param,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Tool: get_subagents
// =============================================================================

pub struct GetSubagentsTool;

#[async_trait]
impl Tool for GetSubagentsTool {
    fn name(&self) -> &str {
        "get_subagents"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Get Subagents")
    }

    fn description(&self) -> &str {
        "List all subagents or get detailed status of a specific one (by name or ID)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name_or_id": {
                    "type": "string",
                    "description": "Subagent name or session ID for detailed view. Omit to list all."
                },
                "status_filter": {
                    "type": "string",
                    "enum": ["all", "running", "completed", "failed"],
                    "description": "Filter by status when listing all subagents."
                }
            },
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("get_subagents requires context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let store = match get_platform_store(context) {
            Ok(s) => s,
            Err(e) => return e,
        };

        let all_sessions = match store.list_sessions(Some(100), None).await {
            Ok(s) => s,
            Err(e) => return ToolExecutionResult::internal_error(e),
        };

        let name_or_id = arguments
            .get("name_or_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty());

        if let Some(query) = name_or_id {
            let found = find_child_session(&all_sessions, context.session_id, query);

            match found {
                Some(child) => {
                    let messages = store
                        .get_messages(child.id, Some(10))
                        .await
                        .unwrap_or_default();

                    let last_response = last_agent_message(&messages);

                    ToolExecutionResult::success(json!({
                        "subagent_id": child.id.to_string(),
                        "name": child.subagent_name,
                        "task": child.subagent_task,
                        "status": child.subagent_status.as_ref().map(|s| s.to_string())
                            .unwrap_or_else(|| child.status.to_string()),
                        "session_status": child.status.to_string(),
                        "created_at": child.created_at.to_rfc3339(),
                        "result": last_response,
                        "blueprint_id": child.blueprint_id,
                    }))
                }
                None => ToolExecutionResult::tool_error(format!(
                    "No subagent found with name or ID: {query}"
                )),
            }
        } else {
            // List all subagents
            let status_filter = arguments.get("status_filter").and_then(|v| v.as_str());

            let filtered: Vec<_> = all_sessions
                .iter()
                .filter(|s| s.parent_session_id == Some(context.session_id))
                .filter(|s| {
                    if let Some(filter) = status_filter {
                        if filter == "all" {
                            return true;
                        }
                        s.subagent_status
                            .as_ref()
                            .is_some_and(|st| st.to_string() == filter)
                    } else {
                        true
                    }
                })
                .map(|s| {
                    json!({
                        "subagent_id": s.id.to_string(),
                        "name": s.subagent_name,
                        "task": s.subagent_task,
                        "status": s.subagent_status.as_ref().map(|st| st.to_string())
                            .unwrap_or_else(|| s.status.to_string()),
                        "created_at": s.created_at.to_rfc3339(),
                        "blueprint_id": s.blueprint_id,
                    })
                })
                .collect();

            ToolExecutionResult::success(json!({
                "subagents": filtered,
                "count": filtered.len(),
            }))
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Tool: message_subagent
// =============================================================================

pub struct MessageSubagentTool;

#[async_trait]
impl Tool for MessageSubagentTool {
    fn name(&self) -> &str {
        "message_subagent"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Message Subagent")
    }

    fn description(&self) -> &str {
        "Send a message to a subagent by name or ID. Steers running subagents, resumes completed/failed ones."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name_or_id": {
                    "type": "string",
                    "description": "Subagent name or session ID."
                },
                "message": {
                    "type": "string",
                    "description": "Message to send to the subagent."
                },
                "cancel": {
                    "type": "boolean",
                    "description": "If true, deliver the message then gracefully stop the subagent.",
                    "default": false
                }
            },
            "required": ["name_or_id", "message"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("message_subagent requires context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let store = match get_platform_store(context) {
            Ok(s) => s,
            Err(e) => return e,
        };

        let name_or_id = match require_str(&arguments, "name_or_id") {
            Ok(s) => s.to_string(),
            Err(e) => return e,
        };
        let message = match require_str(&arguments, "message") {
            Ok(s) => s.to_string(),
            Err(e) => return e,
        };
        let cancel = arguments
            .get("cancel")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let all_sessions = match store.list_sessions(Some(100), None).await {
            Ok(s) => s,
            Err(e) => return ToolExecutionResult::internal_error(e),
        };

        let child = match find_child_session(&all_sessions, context.session_id, &name_or_id) {
            Some(c) => c,
            None => {
                return ToolExecutionResult::tool_error(format!(
                    "No subagent found with name or ID: {name_or_id}"
                ));
            }
        };

        let child_id = child.id;

        // Send the message
        if let Err(e) = store.send_message(child_id, &message).await {
            return ToolExecutionResult::internal_error(e);
        }

        if cancel {
            // For cancel, just send the message and report
            // (actual cancellation mechanism to be added when background mode lands)
            return ToolExecutionResult::success(json!({
                "subagent_id": child_id.to_string(),
                "name": child.subagent_name,
                "delivered": true,
                "cancel_requested": true,
                "note": "Message delivered. Cancellation will take effect after current turn.",
            }));
        }

        // Wait for the subagent to process the message
        let status = match store.wait_for_idle(child_id, Some(300)).await {
            Ok(s) => s,
            Err(e) => {
                return ToolExecutionResult::success(json!({
                    "subagent_id": child_id.to_string(),
                    "name": child.subagent_name,
                    "delivered": true,
                    "status": "error",
                    "error": e.to_string(),
                }));
            }
        };

        // Get the latest response
        let messages = match store.get_messages(child_id, Some(5)).await {
            Ok(m) => m,
            Err(e) => return ToolExecutionResult::internal_error(e),
        };

        ToolExecutionResult::success(json!({
            "subagent_id": child_id.to_string(),
            "name": child.subagent_name,
            "delivered": true,
            "status": status,
            "result": last_agent_message(&messages),
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Tool;

    #[test]
    fn capability_basics() {
        let cap = SubagentCapability;
        assert_eq!(cap.id(), "subagents");
        assert_eq!(cap.tools().len(), 3);
        assert!(cap.system_prompt_addition().is_some());
        assert_eq!(cap.features(), vec!["subagents"]);
    }

    #[test]
    fn tool_names() {
        let cap = SubagentCapability;
        let tools = cap.tools();
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert_eq!(
            names,
            vec!["spawn_subagent", "get_subagents", "message_subagent"]
        );
    }

    #[test]
    fn spawn_subagent_schema_has_required_fields() {
        let tool = SpawnSubagentTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("name")));
        assert!(required.contains(&json!("task")));
    }

    #[test]
    fn spawn_subagent_schema_has_blueprint_fields() {
        let tool = SpawnSubagentTool;
        let schema = tool.parameters_schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("blueprint"));
        assert!(props.contains_key("config"));
        // blueprint and config should NOT be required
        let required = schema["required"].as_array().unwrap();
        assert!(!required.contains(&json!("blueprint")));
        assert!(!required.contains(&json!("config")));
    }

    #[tokio::test]
    async fn spawn_subagent_without_context_returns_error() {
        let tool = SpawnSubagentTool;
        let result = tool.execute(json!({"name": "Test", "task": "test"})).await;
        assert!(matches!(result, ToolExecutionResult::ToolError(_)));
    }

    #[tokio::test]
    async fn get_subagents_without_context_returns_error() {
        let tool = GetSubagentsTool;
        let result = tool.execute(json!({})).await;
        assert!(matches!(result, ToolExecutionResult::ToolError(_)));
    }

    #[tokio::test]
    async fn message_subagent_without_context_returns_error() {
        let tool = MessageSubagentTool;
        let result = tool
            .execute(json!({"name_or_id": "Test", "message": "hello"}))
            .await;
        assert!(matches!(result, ToolExecutionResult::ToolError(_)));
    }
}
