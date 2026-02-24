// Platform Management capability
// THREAT[TM-AGENT-017]: Agents with this capability can manage org-wide entities
//
// Decision: Four tools grouped by entity (harnesses, agents, sessions, session_interact)
// Decision: Each tool uses an `operation` parameter for CRUD dispatch
// Decision: All results include UI links via PlatformStore::base_url()
// Decision: get_messages defaults to last 10; wait_for_idle defaults to 120s timeout

use super::{Capability, CapabilityStatus};
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde_json::{Value, json};

// =============================================================================
// Capability
// =============================================================================

pub struct PlatformManagementCapability;

impl Capability for PlatformManagementCapability {
    fn id(&self) -> &str {
        "platform_management"
    }

    fn name(&self) -> &str {
        "Platform Management"
    }

    fn description(&self) -> &str {
        "Tools to manage harnesses, agents, and sessions. Create, list, update, delete entities and interact with sessions programmatically."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("settings-2")
    }

    fn category(&self) -> Option<&str> {
        Some("Platform")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"You have platform management tools to manage Everruns entities:

`manage_harnesses` - Harness CRUD operations:
- operation: "list" - List all harnesses
- operation: "get" - Get a harness by ID (requires: harness_id)
- operation: "create" - Create a harness (requires: name, system_prompt; optional: description, capabilities)
- operation: "update" - Update a harness (requires: harness_id; optional: name, description, system_prompt)
- operation: "delete" - Archive a harness (requires: harness_id)
- operation: "copy" - Copy a harness (requires: harness_id; optional: new_name)

`manage_agents` - Agent CRUD operations:
- operation: "list" - List all agents
- operation: "get" - Get an agent by ID (requires: agent_id)
- operation: "create" - Create an agent (requires: name, system_prompt; optional: description, capabilities)
- operation: "update" - Update an agent (requires: agent_id; optional: name, description, system_prompt)
- operation: "delete" - Archive an agent (requires: agent_id)

`manage_sessions` - Session operations:
- operation: "list" - List sessions (optional: limit, agent_id)
- operation: "create" - Create a session (requires: harness_id; optional: agent_id, title)
- operation: "get" - Get a session by ID (requires: session_id)
- operation: "delete" - Archive a session (requires: session_id)

`session_interact` - Session messaging and turn management:
- operation: "send_message" - Send a user message to a session (requires: session_id, content)
- operation: "get_messages" - Get messages from a session (requires: session_id; optional: limit, default 10)
- operation: "wait_for_idle" - Wait for turn to complete (requires: session_id; optional: timeout_secs, default 120)

All results include UI links to view entities in the web interface."#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(ManageHarnessesTool),
            Box::new(ManageAgentsTool),
            Box::new(ManageSessionsTool),
            Box::new(SessionInteractTool),
        ]
    }
}

// =============================================================================
// Helper: extract platform_store from context
// =============================================================================

fn get_platform_store(
    context: &ToolContext,
) -> Result<&dyn crate::platform_store::PlatformStore, ToolExecutionResult> {
    match &context.platform_store {
        Some(store) => Ok(store.as_ref()),
        None => Err(ToolExecutionResult::tool_error(
            "Platform management not available in this context. Ensure the platform_management capability is enabled.",
        )),
    }
}

fn get_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolExecutionResult> {
    get_str(args, key).ok_or_else(|| {
        ToolExecutionResult::tool_error(format!("Missing required parameter: {key}"))
    })
}

// =============================================================================
// Tool: manage_harnesses
// =============================================================================

pub struct ManageHarnessesTool;

#[async_trait]
impl Tool for ManageHarnessesTool {
    fn name(&self) -> &str {
        "manage_harnesses"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Manage Harnesses")
    }

    fn description(&self) -> &str {
        "CRUD operations for harnesses: list, get, create, update, delete, copy."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["list", "get", "create", "update", "delete", "copy"],
                    "description": "The operation to perform"
                },
                "harness_id": {
                    "type": "string",
                    "description": "Harness ID (required for get, update, delete, copy)"
                },
                "name": {
                    "type": "string",
                    "description": "Harness name (required for create)"
                },
                "new_name": {
                    "type": "string",
                    "description": "New name when copying a harness"
                },
                "description": {
                    "type": "string",
                    "description": "Harness description"
                },
                "system_prompt": {
                    "type": "string",
                    "description": "System prompt (required for create)"
                },
                "capabilities": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of capability IDs"
                }
            },
            "required": ["operation"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "manage_harnesses requires context. This tool must be executed with session context.",
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

        let operation = match require_str(&arguments, "operation") {
            Ok(op) => op,
            Err(e) => return e,
        };

        let base_url = store.base_url();

        match operation {
            "list" => match store.list_harnesses().await {
                Ok(harnesses) => {
                    let items: Vec<Value> = harnesses
                        .iter()
                        .map(|h| {
                            json!({
                                "id": h.id.to_string(),
                                "name": h.name,
                                "description": h.description,
                                "status": format!("{:?}", h.status),
                                "capabilities": h.capabilities.iter().map(|c| c.capability_id().to_string()).collect::<Vec<_>>(),
                                "tags": h.tags,
                                "ui_link": format!("{}/harnesses/{}", base_url, h.id),
                            })
                        })
                        .collect();
                    ToolExecutionResult::success(json!({"harnesses": items, "count": items.len()}))
                }
                Err(e) => ToolExecutionResult::tool_error(format!("Failed to list harnesses: {e}")),
            },

            "get" => {
                let id_str = match require_str(&arguments, "harness_id") {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                let id = match id_str.parse::<crate::typed_id::HarnessId>() {
                    Ok(id) => id,
                    Err(_) => {
                        return ToolExecutionResult::tool_error(format!(
                            "Invalid harness_id: {id_str}"
                        ));
                    }
                };
                match store.get_harness(id).await {
                    Ok(Some(h)) => ToolExecutionResult::success(json!({
                        "id": h.id.to_string(),
                        "name": h.name,
                        "description": h.description,
                        "system_prompt": h.system_prompt,
                        "status": format!("{:?}", h.status),
                        "capabilities": h.capabilities.iter().map(|c| c.capability_id().to_string()).collect::<Vec<_>>(),
                        "tags": h.tags,
                        "ui_link": format!("{}/harnesses/{}", base_url, h.id),
                    })),
                    Ok(None) => {
                        ToolExecutionResult::tool_error(format!("Harness not found: {id_str}"))
                    }
                    Err(e) => {
                        ToolExecutionResult::tool_error(format!("Failed to get harness: {e}"))
                    }
                }
            }

            "create" => {
                let name = match require_str(&arguments, "name") {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                let system_prompt = match require_str(&arguments, "system_prompt") {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                let description = get_str(&arguments, "description");
                let capabilities: Vec<String> = arguments
                    .get("capabilities")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                match store
                    .create_harness(name, description, system_prompt, &capabilities)
                    .await
                {
                    Ok(h) => ToolExecutionResult::success(json!({
                        "id": h.id.to_string(),
                        "name": h.name,
                        "description": h.description,
                        "status": format!("{:?}", h.status),
                        "ui_link": format!("{}/harnesses/{}", base_url, h.id),
                        "message": "Harness created successfully"
                    })),
                    Err(e) => {
                        ToolExecutionResult::tool_error(format!("Failed to create harness: {e}"))
                    }
                }
            }

            "update" => {
                let id_str = match require_str(&arguments, "harness_id") {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                let id = match id_str.parse::<crate::typed_id::HarnessId>() {
                    Ok(id) => id,
                    Err(_) => {
                        return ToolExecutionResult::tool_error(format!(
                            "Invalid harness_id: {id_str}"
                        ));
                    }
                };
                let name = get_str(&arguments, "name");
                let description = get_str(&arguments, "description");
                let system_prompt = get_str(&arguments, "system_prompt");
                match store
                    .update_harness(id, name, description, system_prompt)
                    .await
                {
                    Ok(h) => ToolExecutionResult::success(json!({
                        "id": h.id.to_string(),
                        "name": h.name,
                        "description": h.description,
                        "status": format!("{:?}", h.status),
                        "ui_link": format!("{}/harnesses/{}", base_url, h.id),
                        "message": "Harness updated successfully"
                    })),
                    Err(e) => {
                        ToolExecutionResult::tool_error(format!("Failed to update harness: {e}"))
                    }
                }
            }

            "delete" => {
                let id_str = match require_str(&arguments, "harness_id") {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                let id = match id_str.parse::<crate::typed_id::HarnessId>() {
                    Ok(id) => id,
                    Err(_) => {
                        return ToolExecutionResult::tool_error(format!(
                            "Invalid harness_id: {id_str}"
                        ));
                    }
                };
                match store.delete_harness(id).await {
                    Ok(()) => ToolExecutionResult::success(json!({
                        "harness_id": id_str,
                        "message": "Harness archived successfully"
                    })),
                    Err(e) => {
                        ToolExecutionResult::tool_error(format!("Failed to delete harness: {e}"))
                    }
                }
            }

            "copy" => {
                let id_str = match require_str(&arguments, "harness_id") {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                let id = match id_str.parse::<crate::typed_id::HarnessId>() {
                    Ok(id) => id,
                    Err(_) => {
                        return ToolExecutionResult::tool_error(format!(
                            "Invalid harness_id: {id_str}"
                        ));
                    }
                };
                let new_name = get_str(&arguments, "new_name");
                match store.copy_harness(id, new_name).await {
                    Ok(h) => ToolExecutionResult::success(json!({
                        "id": h.id.to_string(),
                        "name": h.name,
                        "description": h.description,
                        "status": format!("{:?}", h.status),
                        "ui_link": format!("{}/harnesses/{}", base_url, h.id),
                        "source_harness_id": id_str,
                        "message": "Harness copied successfully"
                    })),
                    Err(e) => {
                        ToolExecutionResult::tool_error(format!("Failed to copy harness: {e}"))
                    }
                }
            }

            _ => ToolExecutionResult::tool_error(format!(
                "Unknown operation: {operation}. Valid: list, get, create, update, delete, copy"
            )),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Tool: manage_agents
// =============================================================================

pub struct ManageAgentsTool;

#[async_trait]
impl Tool for ManageAgentsTool {
    fn name(&self) -> &str {
        "manage_agents"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Manage Agents")
    }

    fn description(&self) -> &str {
        "CRUD operations for agents: list, get, create, update, delete."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["list", "get", "create", "update", "delete"],
                    "description": "The operation to perform"
                },
                "agent_id": {
                    "type": "string",
                    "description": "Agent ID (required for get, update, delete)"
                },
                "name": {
                    "type": "string",
                    "description": "Agent name (required for create)"
                },
                "description": {
                    "type": "string",
                    "description": "Agent description"
                },
                "system_prompt": {
                    "type": "string",
                    "description": "System prompt (required for create)"
                },
                "capabilities": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "List of capability IDs"
                }
            },
            "required": ["operation"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "manage_agents requires context. This tool must be executed with session context.",
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

        let operation = match require_str(&arguments, "operation") {
            Ok(op) => op,
            Err(e) => return e,
        };

        let base_url = store.base_url();

        match operation {
            "list" => match store.list_agents().await {
                Ok(agents) => {
                    let items: Vec<Value> = agents
                        .iter()
                        .map(|a| {
                            json!({
                                "id": a.public_id.to_string(),
                                "name": a.name,
                                "description": a.description,
                                "status": format!("{:?}", a.status),
                                "capabilities": a.capabilities.iter().map(|c| c.capability_id().to_string()).collect::<Vec<_>>(),
                                "tags": a.tags,
                                "ui_link": format!("{}/agents/{}", base_url, a.public_id),
                            })
                        })
                        .collect();
                    ToolExecutionResult::success(json!({"agents": items, "count": items.len()}))
                }
                Err(e) => ToolExecutionResult::tool_error(format!("Failed to list agents: {e}")),
            },

            "get" => {
                let id_str = match require_str(&arguments, "agent_id") {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                let id = match id_str.parse::<crate::typed_id::AgentId>() {
                    Ok(id) => id,
                    Err(_) => {
                        return ToolExecutionResult::tool_error(format!(
                            "Invalid agent_id: {id_str}"
                        ));
                    }
                };
                match store.get_agent_by_id(id).await {
                    Ok(Some(a)) => ToolExecutionResult::success(json!({
                        "id": a.public_id.to_string(),
                        "name": a.name,
                        "description": a.description,
                        "system_prompt": a.system_prompt,
                        "status": format!("{:?}", a.status),
                        "capabilities": a.capabilities.iter().map(|c| c.capability_id().to_string()).collect::<Vec<_>>(),
                        "tags": a.tags,
                        "ui_link": format!("{}/agents/{}", base_url, a.public_id),
                    })),
                    Ok(None) => {
                        ToolExecutionResult::tool_error(format!("Agent not found: {id_str}"))
                    }
                    Err(e) => ToolExecutionResult::tool_error(format!("Failed to get agent: {e}")),
                }
            }

            "create" => {
                let name = match require_str(&arguments, "name") {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                let system_prompt = match require_str(&arguments, "system_prompt") {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                let description = get_str(&arguments, "description");
                let capabilities: Vec<String> = arguments
                    .get("capabilities")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                match store
                    .create_agent(name, description, system_prompt, &capabilities)
                    .await
                {
                    Ok(a) => ToolExecutionResult::success(json!({
                        "id": a.public_id.to_string(),
                        "name": a.name,
                        "description": a.description,
                        "status": format!("{:?}", a.status),
                        "ui_link": format!("{}/agents/{}", base_url, a.public_id),
                        "message": "Agent created successfully"
                    })),
                    Err(e) => {
                        ToolExecutionResult::tool_error(format!("Failed to create agent: {e}"))
                    }
                }
            }

            "update" => {
                let id_str = match require_str(&arguments, "agent_id") {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                let id = match id_str.parse::<crate::typed_id::AgentId>() {
                    Ok(id) => id,
                    Err(_) => {
                        return ToolExecutionResult::tool_error(format!(
                            "Invalid agent_id: {id_str}"
                        ));
                    }
                };
                let name = get_str(&arguments, "name");
                let description = get_str(&arguments, "description");
                let system_prompt = get_str(&arguments, "system_prompt");
                match store
                    .update_agent(id, name, description, system_prompt)
                    .await
                {
                    Ok(a) => ToolExecutionResult::success(json!({
                        "id": a.public_id.to_string(),
                        "name": a.name,
                        "description": a.description,
                        "status": format!("{:?}", a.status),
                        "ui_link": format!("{}/agents/{}", base_url, a.public_id),
                        "message": "Agent updated successfully"
                    })),
                    Err(e) => {
                        ToolExecutionResult::tool_error(format!("Failed to update agent: {e}"))
                    }
                }
            }

            "delete" => {
                let id_str = match require_str(&arguments, "agent_id") {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                let id = match id_str.parse::<crate::typed_id::AgentId>() {
                    Ok(id) => id,
                    Err(_) => {
                        return ToolExecutionResult::tool_error(format!(
                            "Invalid agent_id: {id_str}"
                        ));
                    }
                };
                match store.delete_agent(id).await {
                    Ok(()) => ToolExecutionResult::success(json!({
                        "agent_id": id_str,
                        "message": "Agent archived successfully"
                    })),
                    Err(e) => {
                        ToolExecutionResult::tool_error(format!("Failed to delete agent: {e}"))
                    }
                }
            }

            _ => ToolExecutionResult::tool_error(format!(
                "Unknown operation: {operation}. Valid: list, get, create, update, delete"
            )),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Tool: manage_sessions
// =============================================================================

pub struct ManageSessionsTool;

#[async_trait]
impl Tool for ManageSessionsTool {
    fn name(&self) -> &str {
        "manage_sessions"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Manage Sessions")
    }

    fn description(&self) -> &str {
        "Session operations: list, create, get, delete."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["list", "create", "get", "delete"],
                    "description": "The operation to perform"
                },
                "session_id": {
                    "type": "string",
                    "description": "Session ID (required for get, delete)"
                },
                "harness_id": {
                    "type": "string",
                    "description": "Harness ID (required for create)"
                },
                "agent_id": {
                    "type": "string",
                    "description": "Agent ID (optional for create and list)"
                },
                "title": {
                    "type": "string",
                    "description": "Session title (optional for create)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max number of sessions to return (default 20)"
                }
            },
            "required": ["operation"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "manage_sessions requires context. This tool must be executed with session context.",
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

        let operation = match require_str(&arguments, "operation") {
            Ok(op) => op,
            Err(e) => return e,
        };

        let base_url = store.base_url();

        match operation {
            "list" => {
                let limit = arguments
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize);
                let agent_id = get_str(&arguments, "agent_id")
                    .and_then(|s| s.parse::<crate::typed_id::AgentId>().ok());
                match store.list_sessions(limit, agent_id).await {
                    Ok(sessions) => {
                        let items: Vec<Value> = sessions
                            .iter()
                            .map(|s| {
                                json!({
                                    "id": s.id.to_string(),
                                    "title": s.title,
                                    "status": format!("{:?}", s.status),
                                    "agent_id": s.agent_id.as_ref().map(|a| a.to_string()),
                                    "harness_id": s.harness_id.to_string(),
                                    "created_at": s.created_at.to_rfc3339(),
                                    "preview": s.preview,
                                    "ui_link": format!("{}/sessions/{}/chat", base_url, s.id),
                                })
                            })
                            .collect();
                        ToolExecutionResult::success(
                            json!({"sessions": items, "count": items.len()}),
                        )
                    }
                    Err(e) => {
                        ToolExecutionResult::tool_error(format!("Failed to list sessions: {e}"))
                    }
                }
            }

            "create" => {
                let harness_id_str = match require_str(&arguments, "harness_id") {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                let harness_id = match harness_id_str.parse::<crate::typed_id::HarnessId>() {
                    Ok(id) => id,
                    Err(_) => {
                        return ToolExecutionResult::tool_error(format!(
                            "Invalid harness_id: {harness_id_str}"
                        ));
                    }
                };
                let agent_id = get_str(&arguments, "agent_id")
                    .and_then(|s| s.parse::<crate::typed_id::AgentId>().ok());
                let title = get_str(&arguments, "title");
                match store.create_session(harness_id, agent_id, title).await {
                    Ok(s) => ToolExecutionResult::success(json!({
                        "id": s.id.to_string(),
                        "title": s.title,
                        "status": format!("{:?}", s.status),
                        "harness_id": s.harness_id.to_string(),
                        "agent_id": s.agent_id.as_ref().map(|a| a.to_string()),
                        "ui_link": format!("{}/sessions/{}/chat", base_url, s.id),
                        "message": "Session created successfully"
                    })),
                    Err(e) => {
                        ToolExecutionResult::tool_error(format!("Failed to create session: {e}"))
                    }
                }
            }

            "get" => {
                let id_str = match require_str(&arguments, "session_id") {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                let id = match id_str.parse::<crate::typed_id::SessionId>() {
                    Ok(id) => id,
                    Err(_) => {
                        return ToolExecutionResult::tool_error(format!(
                            "Invalid session_id: {id_str}"
                        ));
                    }
                };
                match store.get_session_by_id(id).await {
                    Ok(Some(s)) => ToolExecutionResult::success(json!({
                        "id": s.id.to_string(),
                        "title": s.title,
                        "status": format!("{:?}", s.status),
                        "agent_id": s.agent_id.as_ref().map(|a| a.to_string()),
                        "harness_id": s.harness_id.to_string(),
                        "created_at": s.created_at.to_rfc3339(),
                        "preview": s.preview,
                        "output_preview": s.output_preview,
                        "ui_link": format!("{}/sessions/{}/chat", base_url, s.id),
                    })),
                    Ok(None) => {
                        ToolExecutionResult::tool_error(format!("Session not found: {id_str}"))
                    }
                    Err(e) => {
                        ToolExecutionResult::tool_error(format!("Failed to get session: {e}"))
                    }
                }
            }

            "delete" => {
                let id_str = match require_str(&arguments, "session_id") {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                let id = match id_str.parse::<crate::typed_id::SessionId>() {
                    Ok(id) => id,
                    Err(_) => {
                        return ToolExecutionResult::tool_error(format!(
                            "Invalid session_id: {id_str}"
                        ));
                    }
                };
                match store.delete_session(id).await {
                    Ok(()) => ToolExecutionResult::success(json!({
                        "session_id": id_str,
                        "message": "Session archived successfully"
                    })),
                    Err(e) => {
                        ToolExecutionResult::tool_error(format!("Failed to delete session: {e}"))
                    }
                }
            }

            _ => ToolExecutionResult::tool_error(format!(
                "Unknown operation: {operation}. Valid: list, create, get, delete"
            )),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Tool: session_interact
// =============================================================================

pub struct SessionInteractTool;

#[async_trait]
impl Tool for SessionInteractTool {
    fn name(&self) -> &str {
        "session_interact"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Session Interact")
    }

    fn description(&self) -> &str {
        "Interact with sessions: send messages, get messages, wait for turn completion."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["send_message", "get_messages", "wait_for_idle"],
                    "description": "The operation to perform"
                },
                "session_id": {
                    "type": "string",
                    "description": "Session ID (required for all operations)"
                },
                "content": {
                    "type": "string",
                    "description": "Message content (required for send_message)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max messages to return (default 10, for get_messages)"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Timeout in seconds (default 120, for wait_for_idle)"
                }
            },
            "required": ["operation", "session_id"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "session_interact requires context. This tool must be executed with session context.",
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

        let operation = match require_str(&arguments, "operation") {
            Ok(op) => op,
            Err(e) => return e,
        };

        let session_id_str = match require_str(&arguments, "session_id") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let session_id = match session_id_str.parse::<crate::typed_id::SessionId>() {
            Ok(id) => id,
            Err(_) => {
                return ToolExecutionResult::tool_error(format!(
                    "Invalid session_id: {session_id_str}"
                ));
            }
        };

        let base_url = store.base_url();

        match operation {
            "send_message" => {
                let content = match require_str(&arguments, "content") {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                match store.send_message(session_id, content).await {
                    Ok(()) => ToolExecutionResult::success(json!({
                        "session_id": session_id_str,
                        "message": "Message sent successfully. Use wait_for_idle to wait for the agent response.",
                        "ui_link": format!("{}/sessions/{}/chat", base_url, session_id),
                    })),
                    Err(e) => {
                        ToolExecutionResult::tool_error(format!("Failed to send message: {e}"))
                    }
                }
            }

            "get_messages" => {
                let limit = arguments
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize);
                match store.get_messages(session_id, limit).await {
                    Ok(messages) => {
                        let items: Vec<Value> = messages
                            .iter()
                            .map(|m| {
                                json!({
                                    "role": m.role,
                                    "content": m.content,
                                    "created_at": m.created_at.to_rfc3339(),
                                })
                            })
                            .collect();
                        ToolExecutionResult::success(json!({
                            "messages": items,
                            "count": items.len(),
                            "session_id": session_id_str,
                            "ui_link": format!("{}/sessions/{}/chat", base_url, session_id),
                        }))
                    }
                    Err(e) => {
                        ToolExecutionResult::tool_error(format!("Failed to get messages: {e}"))
                    }
                }
            }

            "wait_for_idle" => {
                let timeout_secs = arguments.get("timeout_secs").and_then(|v| v.as_u64());
                match store.wait_for_idle(session_id, timeout_secs).await {
                    Ok(status) => ToolExecutionResult::success(json!({
                        "session_id": session_id_str,
                        "status": status,
                        "ui_link": format!("{}/sessions/{}/chat", base_url, session_id),
                    })),
                    Err(e) => {
                        ToolExecutionResult::tool_error(format!("Failed waiting for idle: {e}"))
                    }
                }
            }

            _ => ToolExecutionResult::tool_error(format!(
                "Unknown operation: {operation}. Valid: send_message, get_messages, wait_for_idle"
            )),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentCapabilityConfig;
    use crate::agent::{Agent, AgentStatus};
    use crate::error::Result;
    use crate::harness::{Harness, HarnessStatus};
    use crate::platform_store::{PlatformMessage, PlatformStore};
    use crate::session::{Session, SessionStatus};
    use crate::typed_id::{AgentId, HarnessId, SessionId};
    use async_trait::async_trait;
    use chrono::Utc;
    use std::sync::Arc;

    fn build_harness(name: &str) -> Harness {
        Harness {
            id: HarnessId::new(),
            name: name.to_string(),
            description: Some("test harness".to_string()),
            system_prompt: "You are helpful.".to_string(),
            default_model_id: None,
            tags: vec![],
            capabilities: vec![AgentCapabilityConfig::new("session")],
            status: HarnessStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn build_agent(name: &str) -> Agent {
        Agent {
            public_id: AgentId::new(),
            internal_id: uuid::Uuid::now_v7(),
            name: name.to_string(),
            description: Some("test agent".to_string()),
            system_prompt: "You are helpful.".to_string(),
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            tools: vec![],
            status: AgentStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            usage: None,
        }
    }

    fn build_session(title: &str) -> Session {
        Session {
            id: SessionId::new(),
            organization_id: "org_00000000000000000000000000000001".to_string(),
            harness_id: HarnessId::new(),
            agent_id: None,
            title: Some(title.to_string()),
            preview: None,
            output_preview: None,
            tags: vec![],
            model_id: None,
            capabilities: vec![],
            tools: vec![],
            status: SessionStatus::Idle,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            started_at: None,
            finished_at: None,
            usage: None,
            is_pinned: None,
            active_schedule_count: None,
            features: vec![],
        }
    }

    struct MockPlatformStore {
        harness: Harness,
        agent: Agent,
        session: Session,
    }

    impl MockPlatformStore {
        fn new() -> Self {
            Self {
                harness: build_harness("Test Harness"),
                agent: build_agent("Test Agent"),
                session: build_session("Test Session"),
            }
        }
    }

    #[async_trait]
    impl PlatformStore for MockPlatformStore {
        async fn list_harnesses(&self) -> Result<Vec<Harness>> {
            Ok(vec![self.harness.clone()])
        }
        async fn get_harness(&self, _id: HarnessId) -> Result<Option<Harness>> {
            Ok(Some(self.harness.clone()))
        }
        async fn create_harness(
            &self,
            name: &str,
            _desc: Option<&str>,
            _prompt: &str,
            _caps: &[String],
        ) -> Result<Harness> {
            let mut h = self.harness.clone();
            h.name = name.to_string();
            Ok(h)
        }
        async fn update_harness(
            &self,
            _id: HarnessId,
            name: Option<&str>,
            _desc: Option<&str>,
            _prompt: Option<&str>,
        ) -> Result<Harness> {
            let mut h = self.harness.clone();
            if let Some(n) = name {
                h.name = n.to_string();
            }
            Ok(h)
        }
        async fn delete_harness(&self, _id: HarnessId) -> Result<()> {
            Ok(())
        }
        async fn copy_harness(&self, _id: HarnessId, new_name: Option<&str>) -> Result<Harness> {
            let mut h = self.harness.clone();
            h.id = HarnessId::new();
            h.name = new_name.unwrap_or("Copy").to_string();
            Ok(h)
        }
        async fn list_agents(&self) -> Result<Vec<Agent>> {
            Ok(vec![self.agent.clone()])
        }
        async fn get_agent_by_id(&self, _id: AgentId) -> Result<Option<Agent>> {
            Ok(Some(self.agent.clone()))
        }
        async fn create_agent(
            &self,
            name: &str,
            _desc: Option<&str>,
            _prompt: &str,
            _caps: &[String],
        ) -> Result<Agent> {
            let mut a = self.agent.clone();
            a.name = name.to_string();
            Ok(a)
        }
        async fn update_agent(
            &self,
            _id: AgentId,
            name: Option<&str>,
            _desc: Option<&str>,
            _prompt: Option<&str>,
        ) -> Result<Agent> {
            let mut a = self.agent.clone();
            if let Some(n) = name {
                a.name = n.to_string();
            }
            Ok(a)
        }
        async fn delete_agent(&self, _id: AgentId) -> Result<()> {
            Ok(())
        }
        async fn list_sessions(
            &self,
            _limit: Option<usize>,
            _agent_id: Option<AgentId>,
        ) -> Result<Vec<Session>> {
            Ok(vec![self.session.clone()])
        }
        async fn create_session(
            &self,
            _hid: HarnessId,
            _aid: Option<AgentId>,
            title: Option<&str>,
        ) -> Result<Session> {
            let mut s = self.session.clone();
            s.title = title.map(|t| t.to_string());
            Ok(s)
        }
        async fn get_session_by_id(&self, _id: SessionId) -> Result<Option<Session>> {
            Ok(Some(self.session.clone()))
        }
        async fn delete_session(&self, _id: SessionId) -> Result<()> {
            Ok(())
        }
        async fn send_message(&self, _id: SessionId, _content: &str) -> Result<()> {
            Ok(())
        }
        async fn get_messages(
            &self,
            _id: SessionId,
            _limit: Option<usize>,
        ) -> Result<Vec<PlatformMessage>> {
            Ok(vec![
                PlatformMessage {
                    role: "user".into(),
                    content: "Hello".into(),
                    created_at: Utc::now(),
                },
                PlatformMessage {
                    role: "agent".into(),
                    content: "Hi!".into(),
                    created_at: Utc::now(),
                },
            ])
        }
        async fn wait_for_idle(&self, _id: SessionId, _t: Option<u64>) -> Result<String> {
            Ok("idle".to_string())
        }
        fn base_url(&self) -> &str {
            "http://localhost:3000"
        }
    }

    fn mock_context() -> ToolContext {
        let store: Arc<dyn PlatformStore> = Arc::new(MockPlatformStore::new());
        let mut ctx = ToolContext::new(SessionId::new());
        ctx.platform_store = Some(store);
        ctx
    }

    #[test]
    fn capability_id_is_platform_management() {
        let cap = PlatformManagementCapability;
        assert_eq!(cap.id(), "platform_management");
        assert_eq!(cap.status(), CapabilityStatus::Available);
    }

    #[test]
    fn capability_provides_four_tools() {
        let cap = PlatformManagementCapability;
        let tools = cap.tools();
        assert_eq!(tools.len(), 4);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"manage_harnesses"));
        assert!(names.contains(&"manage_agents"));
        assert!(names.contains(&"manage_sessions"));
        assert!(names.contains(&"session_interact"));
    }

    #[tokio::test]
    async fn harness_list_returns_harnesses_with_ui_link() {
        let ctx = mock_context();
        let tool = ManageHarnessesTool;
        let result = tool
            .execute_with_context(json!({"operation": "list"}), &ctx)
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["count"], 1);
                let h = v["harnesses"].as_array().unwrap();
                assert!(h[0]["ui_link"].as_str().unwrap().contains("/harnesses/"));
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn harness_create_returns_new_harness() {
        let ctx = mock_context();
        let tool = ManageHarnessesTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "create", "name": "My Harness", "system_prompt": "Be fun!"}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["name"], "My Harness");
                assert!(
                    v["ui_link"]
                        .as_str()
                        .unwrap()
                        .starts_with("http://localhost:3000/harnesses/")
                );
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn harness_copy_returns_copied_harness() {
        let ctx = mock_context();
        let tool = ManageHarnessesTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "copy", "harness_id": HarnessId::new().to_string(), "new_name": "Fun"}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["name"], "Fun");
                assert!(v["message"].as_str().unwrap().contains("copied"));
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn harness_delete_succeeds() {
        let ctx = mock_context();
        let tool = ManageHarnessesTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "delete", "harness_id": HarnessId::new().to_string()}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert!(v["message"].as_str().unwrap().contains("archived"))
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn harness_invalid_operation_returns_error() {
        let ctx = mock_context();
        let tool = ManageHarnessesTool;
        let result = tool
            .execute_with_context(json!({"operation": "explode"}), &ctx)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Unknown operation")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn agent_list_returns_agents() {
        let ctx = mock_context();
        let tool = ManageAgentsTool;
        let result = tool
            .execute_with_context(json!({"operation": "list"}), &ctx)
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["count"], 1);
                assert!(
                    v["agents"].as_array().unwrap()[0]["ui_link"]
                        .as_str()
                        .unwrap()
                        .contains("/agents/")
                );
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn agent_create_returns_new_agent() {
        let ctx = mock_context();
        let tool = ManageAgentsTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "create", "name": "New Agent", "system_prompt": "Be helpful"}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => assert_eq!(v["name"], "New Agent"),
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn session_list_returns_sessions() {
        let ctx = mock_context();
        let tool = ManageSessionsTool;
        let result = tool
            .execute_with_context(json!({"operation": "list"}), &ctx)
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["count"], 1);
                assert!(
                    v["sessions"].as_array().unwrap()[0]["ui_link"]
                        .as_str()
                        .unwrap()
                        .contains("/chat")
                );
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn session_create_returns_new_session() {
        let ctx = mock_context();
        let tool = ManageSessionsTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "create", "harness_id": HarnessId::new().to_string(), "title": "My Session"}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert!(v["ui_link"].as_str().unwrap().contains("/chat"))
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn interact_send_message_succeeds() {
        let ctx = mock_context();
        let tool = SessionInteractTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "send_message", "session_id": SessionId::new().to_string(), "content": "Hi!"}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert!(v["message"].as_str().unwrap().contains("sent"))
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn interact_get_messages_returns_messages() {
        let ctx = mock_context();
        let tool = SessionInteractTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "get_messages", "session_id": SessionId::new().to_string(), "limit": 5}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["count"], 2);
                let msgs = v["messages"].as_array().unwrap();
                assert_eq!(msgs[0]["role"], "user");
                assert_eq!(msgs[1]["role"], "agent");
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn interact_wait_for_idle_succeeds() {
        let ctx = mock_context();
        let tool = SessionInteractTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "wait_for_idle", "session_id": SessionId::new().to_string()}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => assert_eq!(v["status"], "idle"),
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn tool_without_context_returns_error() {
        let tool = ManageHarnessesTool;
        let result = tool.execute(json!({"operation": "list"})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("requires context")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn tool_without_platform_store_returns_error() {
        let ctx = ToolContext::new(SessionId::new());
        let tool = ManageHarnessesTool;
        let result = tool
            .execute_with_context(json!({"operation": "list"}), &ctx)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("not available")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_operation_returns_error() {
        let ctx = mock_context();
        let tool = ManageHarnessesTool;
        let result = tool.execute_with_context(json!({}), &ctx).await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("operation")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn invalid_harness_id_returns_error() {
        let ctx = mock_context();
        let tool = ManageHarnessesTool;
        let result = tool
            .execute_with_context(json!({"operation": "get", "harness_id": "bad"}), &ctx)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Invalid harness_id")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_required_param_returns_error() {
        let ctx = mock_context();
        let tool = ManageHarnessesTool;
        let result = tool
            .execute_with_context(json!({"operation": "create"}), &ctx)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Missing required")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    // =========================================================================
    // Additional negative tests
    // =========================================================================

    #[tokio::test]
    async fn harness_get_succeeds() {
        let ctx = mock_context();
        let tool = ManageHarnessesTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "get", "harness_id": HarnessId::new().to_string()}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["name"], "Test Harness");
                assert!(v["system_prompt"].as_str().is_some());
                assert!(v["ui_link"].as_str().unwrap().contains("/harnesses/"));
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn harness_update_succeeds() {
        let ctx = mock_context();
        let tool = ManageHarnessesTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "update", "harness_id": HarnessId::new().to_string(), "name": "Updated"}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["name"], "Updated");
                assert!(v["message"].as_str().unwrap().contains("updated"));
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn agent_get_succeeds() {
        let ctx = mock_context();
        let tool = ManageAgentsTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "get", "agent_id": AgentId::new().to_string()}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["name"], "Test Agent");
                assert!(v["ui_link"].as_str().unwrap().contains("/agents/"));
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn agent_update_succeeds() {
        let ctx = mock_context();
        let tool = ManageAgentsTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "update", "agent_id": AgentId::new().to_string(), "name": "Renamed"}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["name"], "Renamed");
                assert!(v["message"].as_str().unwrap().contains("updated"));
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn agent_delete_succeeds() {
        let ctx = mock_context();
        let tool = ManageAgentsTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "delete", "agent_id": AgentId::new().to_string()}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert!(v["message"].as_str().unwrap().contains("archived"));
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn agent_invalid_operation_returns_error() {
        let ctx = mock_context();
        let tool = ManageAgentsTool;
        let result = tool
            .execute_with_context(json!({"operation": "clone"}), &ctx)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Unknown operation")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn agent_invalid_id_returns_error() {
        let ctx = mock_context();
        let tool = ManageAgentsTool;
        let result = tool
            .execute_with_context(json!({"operation": "get", "agent_id": "not-valid"}), &ctx)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Invalid agent_id")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn agent_create_missing_name_returns_error() {
        let ctx = mock_context();
        let tool = ManageAgentsTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "create", "system_prompt": "test"}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Missing required")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn session_get_succeeds() {
        let ctx = mock_context();
        let tool = ManageSessionsTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "get", "session_id": SessionId::new().to_string()}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["title"], "Test Session");
                assert!(v["ui_link"].as_str().unwrap().contains("/chat"));
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn session_delete_succeeds() {
        let ctx = mock_context();
        let tool = ManageSessionsTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "delete", "session_id": SessionId::new().to_string()}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert!(v["message"].as_str().unwrap().contains("archived"));
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn session_invalid_operation_returns_error() {
        let ctx = mock_context();
        let tool = ManageSessionsTool;
        let result = tool
            .execute_with_context(json!({"operation": "update"}), &ctx)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Unknown operation")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn session_invalid_id_returns_error() {
        let ctx = mock_context();
        let tool = ManageSessionsTool;
        let result = tool
            .execute_with_context(json!({"operation": "get", "session_id": "nope"}), &ctx)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Invalid session_id")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn session_create_missing_harness_id_returns_error() {
        let ctx = mock_context();
        let tool = ManageSessionsTool;
        let result = tool
            .execute_with_context(json!({"operation": "create"}), &ctx)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Missing required")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn interact_invalid_operation_returns_error() {
        let ctx = mock_context();
        let tool = SessionInteractTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "delete", "session_id": SessionId::new().to_string()}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Unknown operation")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn interact_send_message_missing_content_returns_error() {
        let ctx = mock_context();
        let tool = SessionInteractTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "send_message", "session_id": SessionId::new().to_string()}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Missing required")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn interact_invalid_session_id_returns_error() {
        let ctx = mock_context();
        let tool = SessionInteractTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "get_messages", "session_id": "bad-id"}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Invalid session_id")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn interact_missing_session_id_returns_error() {
        let ctx = mock_context();
        let tool = SessionInteractTool;
        let result = tool
            .execute_with_context(json!({"operation": "get_messages"}), &ctx)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Missing required")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn all_tools_require_context() {
        // All four tools should have requires_context = true
        assert!(ManageHarnessesTool.requires_context());
        assert!(ManageAgentsTool.requires_context());
        assert!(ManageSessionsTool.requires_context());
        assert!(SessionInteractTool.requires_context());
    }

    #[tokio::test]
    async fn all_tools_without_context_return_error() {
        // execute() (no context) should fail for all tools
        for tool_name in [
            "manage_harnesses",
            "manage_agents",
            "manage_sessions",
            "session_interact",
        ] {
            let result = match tool_name {
                "manage_harnesses" => {
                    ManageHarnessesTool
                        .execute(json!({"operation": "list"}))
                        .await
                }
                "manage_agents" => ManageAgentsTool.execute(json!({"operation": "list"})).await,
                "manage_sessions" => {
                    ManageSessionsTool
                        .execute(json!({"operation": "list"}))
                        .await
                }
                "session_interact" => {
                    SessionInteractTool
                        .execute(json!({"operation": "get_messages", "session_id": "x"}))
                        .await
                }
                _ => unreachable!(),
            };
            match result {
                ToolExecutionResult::ToolError(msg) => {
                    assert!(msg.contains("requires context"), "tool {tool_name}: {msg}");
                }
                other => panic!("{tool_name}: expected tool error, got: {other:?}"),
            }
        }
    }

    #[test]
    fn capability_has_system_prompt_addition() {
        let cap = PlatformManagementCapability;
        let prompt = cap.system_prompt_addition().expect("should have prompt");
        assert!(prompt.contains("manage_harnesses"));
        assert!(prompt.contains("manage_agents"));
        assert!(prompt.contains("manage_sessions"));
        assert!(prompt.contains("session_interact"));
    }
}
