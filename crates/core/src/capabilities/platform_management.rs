// Platform Management capability
// THREAT[TM-AGENT-017]: Agents with this capability can manage org-wide entities
//
// Decision: Read/write split — read tools (read_*) return single item by ID or filtered list;
//           write tools (manage_*) perform mutations. Session I/O split into three single-purpose tools.
// Decision: All results include UI links via PlatformStore::base_url()
// Decision: get_messages defaults to last 10; session_read_response defaults to 120s timeout

use super::{Capability, CapabilityStatus};
use crate::tool_types::ToolHints;
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
            r#"Capabilities extend agent/harness functionality. Three types: built-in, MCP servers, and skills. Use `read_capabilities` to discover IDs before creating agents/harnesses. All results include UI links."#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(ReadCapabilitiesTool),
            Box::new(ReadHarnessesTool),
            Box::new(ManageHarnessesTool),
            Box::new(ReadAgentsTool),
            Box::new(ManageAgentsTool),
            Box::new(ReadSessionsTool),
            Box::new(ManageSessionsTool),
            Box::new(SessionSendMessageTool),
            Box::new(SessionReadMessagesTool),
            Box::new(SessionReadResponseTool),
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
    args.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolExecutionResult> {
    get_str(args, key).ok_or_else(|| {
        ToolExecutionResult::tool_error(format!("Missing required parameter: {key}"))
    })
}

// =============================================================================
// Tool: read_harnesses (read-only: get by ID or list all)
// =============================================================================

pub struct ReadHarnessesTool;

#[async_trait]
impl Tool for ReadHarnessesTool {
    fn name(&self) -> &str {
        "read_harnesses"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Read Harnesses")
    }

    fn description(&self) -> &str {
        "Read harnesses by ID or list all. When id is provided returns full detail including system_prompt; otherwise returns summaries."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Optional harness ID to get a single harness with full detail (incl. system_prompt)"
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
        ToolExecutionResult::tool_error(
            "read_harnesses requires context. This tool must be executed with session context.",
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

        let base_url = store.base_url();

        if let Some(id_str) = get_str(&arguments, "id") {
            let id = match id_str.parse::<crate::typed_id::HarnessId>() {
                Ok(id) => id,
                Err(_) => {
                    return ToolExecutionResult::tool_error(format!(
                        "Invalid harness id: {id_str}"
                    ));
                }
            };
            match store.get_harness(id).await {
                Ok(Some(h)) => ToolExecutionResult::success(json!({
                    "id": h.id.to_string(),
                    "name": h.name,
                    "display_name": h.display_name,
                    "description": h.description,
                    "system_prompt": h.system_prompt,
                    "status": format!("{:?}", h.status),
                    "capabilities": h.capabilities.iter().map(|c| c.capability_id().to_string()).collect::<Vec<_>>(),
                    "tags": h.tags,
                    "ui_link": format!("{}/harnesses/{}", base_url, h.id),
                })),
                Ok(None) => ToolExecutionResult::tool_error(format!("Harness not found: {id_str}")),
                Err(e) => ToolExecutionResult::tool_error(format!("Failed to get harness: {e}")),
            }
        } else {
            match store.list_harnesses().await {
                Ok(harnesses) => {
                    let items: Vec<Value> = harnesses
                        .iter()
                        .map(|h| {
                            json!({
                                "id": h.id.to_string(),
                                "name": h.name,
                                "display_name": h.display_name,
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
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Tool: manage_harnesses (mutations: create, update, delete, destroy, copy)
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
        "Harness mutations: create, update, delete, destroy, copy."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["create", "update", "delete", "copy"],
                    "description": "The mutation to perform"
                },
                "harness_id": {
                    "type": "string",
                    "description": "Harness ID (required for update, delete, copy)"
                },
                "name": {
                    "type": "string",
                    "description": "Harness name (required for create, optional for update/copy)"
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
                    "description": "System prompt for the harness. Defaults to 'You are a helpful assistant.' if omitted."
                },
                "parent_harness_id": {
                    "type": ["string", "null"],
                    "description": "Optional parent harness ID. Set to null on update to clear inheritance."
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

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_narration_keys(vec!["operation".to_string(), "name".to_string()])
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
            "create" => {
                let name = match require_str(&arguments, "name") {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                let display_name = match require_str(&arguments, "display_name") {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                let system_prompt =
                    get_str(&arguments, "system_prompt").unwrap_or("You are a helpful assistant.");
                let description = get_str(&arguments, "description");
                let parent_harness_id = match arguments.get("parent_harness_id") {
                    Some(Value::String(id_str)) => {
                        match id_str.parse::<crate::typed_id::HarnessId>() {
                            Ok(id) => Some(id),
                            Err(_) => {
                                return ToolExecutionResult::tool_error(format!(
                                    "Invalid parent_harness_id: {id_str}"
                                ));
                            }
                        }
                    }
                    Some(Value::Null) | None => None,
                    Some(_) => {
                        return ToolExecutionResult::tool_error(
                            "parent_harness_id must be a harness ID string or null",
                        );
                    }
                };
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
                    .create_harness(
                        name,
                        display_name,
                        description,
                        system_prompt,
                        parent_harness_id,
                        &capabilities,
                    )
                    .await
                {
                    Ok(h) => ToolExecutionResult::success(json!({
                        "id": h.id.to_string(),
                        "name": h.name,
                        "display_name": h.display_name,
                        "description": h.description,
                        "parent_harness_id": h.parent_harness_id.map(|id| id.to_string()),
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
                let display_name = get_str(&arguments, "display_name");
                let description = get_str(&arguments, "description");
                let system_prompt = get_str(&arguments, "system_prompt");
                let parent_harness_id = match arguments.get("parent_harness_id") {
                    Some(Value::String(id_str)) => {
                        match id_str.parse::<crate::typed_id::HarnessId>() {
                            Ok(id) => Some(Some(id)),
                            Err(_) => {
                                return ToolExecutionResult::tool_error(format!(
                                    "Invalid parent_harness_id: {id_str}"
                                ));
                            }
                        }
                    }
                    Some(Value::Null) => Some(None),
                    None => None,
                    Some(_) => {
                        return ToolExecutionResult::tool_error(
                            "parent_harness_id must be a harness ID string or null",
                        );
                    }
                };
                match store
                    .update_harness(
                        id,
                        name,
                        display_name,
                        description,
                        system_prompt,
                        parent_harness_id,
                    )
                    .await
                {
                    Ok(h) => ToolExecutionResult::success(json!({
                        "id": h.id.to_string(),
                        "name": h.name,
                        "display_name": h.display_name,
                        "description": h.description,
                        "parent_harness_id": h.parent_harness_id.map(|id| id.to_string()),
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
                        "display_name": h.display_name,
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
                "Unknown operation: {operation}. Valid: create, update, delete, copy"
            )),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Tool: read_agents (read-only: get by ID or list all)
// =============================================================================

pub struct ReadAgentsTool;

#[async_trait]
impl Tool for ReadAgentsTool {
    fn name(&self) -> &str {
        "read_agents"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Read Agents")
    }

    fn description(&self) -> &str {
        "Read agents by ID or list all. When id is provided returns full detail including system_prompt; otherwise returns summaries."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Optional agent ID to get a single agent with full detail (incl. system_prompt)"
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
        ToolExecutionResult::tool_error(
            "read_agents requires context. This tool must be executed with session context.",
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

        let base_url = store.base_url();

        if let Some(id_str) = get_str(&arguments, "id") {
            let id = match id_str.parse::<crate::typed_id::AgentId>() {
                Ok(id) => id,
                Err(_) => {
                    return ToolExecutionResult::tool_error(format!("Invalid agent id: {id_str}"));
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
                Ok(None) => ToolExecutionResult::tool_error(format!("Agent not found: {id_str}")),
                Err(e) => ToolExecutionResult::tool_error(format!("Failed to get agent: {e}")),
            }
        } else {
            match store.list_agents().await {
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
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Tool: manage_agents (mutations: create, update, delete, destroy)
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
        "Agent mutations: create, update, delete, destroy."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["create", "update", "delete"],
                    "description": "The mutation to perform"
                },
                "agent_id": {
                    "type": "string",
                    "description": "Agent ID (required for update, delete)"
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
                    "description": "System prompt for the agent. Defaults to 'You are a helpful assistant.' if omitted."
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

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_narration_keys(vec!["operation".to_string(), "name".to_string()])
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
            "create" => {
                let name = match require_str(&arguments, "name") {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                let system_prompt =
                    get_str(&arguments, "system_prompt").unwrap_or("You are a helpful assistant.");
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
                "Unknown operation: {operation}. Valid: create, update, delete"
            )),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Tool: read_sessions (read-only: get by ID or list/filter)
// =============================================================================

pub struct ReadSessionsTool;

#[async_trait]
impl Tool for ReadSessionsTool {
    fn name(&self) -> &str {
        "read_sessions"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Read Sessions")
    }

    fn description(&self) -> &str {
        "Read sessions by ID or list/filter. When id is provided returns a single session; otherwise returns a filtered list."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Optional session ID to get a single session"
                },
                "agent_id": {
                    "type": "string",
                    "description": "Optional filter by agent"
                },
                "limit": {
                    "type": "integer",
                    "description": "Optional max results for list (default: 20)"
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
        ToolExecutionResult::tool_error(
            "read_sessions requires context. This tool must be executed with session context.",
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

        let base_url = store.base_url();

        if let Some(id_str) = get_str(&arguments, "id") {
            let id = match id_str.parse::<crate::typed_id::SessionId>() {
                Ok(id) => id,
                Err(_) => {
                    return ToolExecutionResult::tool_error(format!(
                        "Invalid session id: {id_str}"
                    ));
                }
            };
            match store.get_session_by_id(id).await {
                Ok(Some(s)) => ToolExecutionResult::success(json!({
                    "id": s.id.to_string(),
                    "organization_id": s.organization_id,
                    "title": s.title,
                    "status": format!("{:?}", s.status),
                    "agent_id": s.agent_id.as_ref().map(|a| a.to_string()),
                    "harness_id": s.harness_id.to_string(),
                    "created_at": s.created_at.to_rfc3339(),
                    "preview": s.preview,
                    "output_preview": s.output_preview,
                    "ui_link": format!("{}/sessions/{}/chat", base_url, s.id),
                })),
                Ok(None) => ToolExecutionResult::tool_error(format!("Session not found: {id_str}")),
                Err(e) => ToolExecutionResult::tool_error(format!("Failed to get session: {e}")),
            }
        } else {
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
                                "organization_id": s.organization_id,
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
                    ToolExecutionResult::success(json!({"sessions": items, "count": items.len()}))
                }
                Err(e) => ToolExecutionResult::tool_error(format!("Failed to list sessions: {e}")),
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Tool: manage_sessions (mutations: create, delete)
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
        "Session mutations: create, delete."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["create", "delete"],
                    "description": "The mutation to perform"
                },
                "session_id": {
                    "type": "string",
                    "description": "Session ID (required for delete)"
                },
                "harness_id": {
                    "type": "string",
                    "description": "Harness ID for the session. If omitted, uses the org's default (Generic) harness."
                },
                "agent_id": {
                    "type": "string",
                    "description": "Agent ID (optional for create)"
                },
                "title": {
                    "type": "string",
                    "description": "Session title (optional for create)"
                },
                "locale": {
                    "type": "string",
                    "description": "Session locale (optional for create, e.g. uk-UA)"
                }
            },
            "required": ["operation"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_narration_keys(vec!["operation".to_string(), "title".to_string()])
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
            "create" => {
                let harness_id = if let Some(harness_id_str) = get_str(&arguments, "harness_id") {
                    match harness_id_str.parse::<crate::typed_id::HarnessId>() {
                        Ok(id) => id,
                        Err(_) => {
                            return ToolExecutionResult::tool_error(format!(
                                "Invalid harness_id: {harness_id_str}"
                            ));
                        }
                    }
                } else {
                    // Fall back to the org's default (Generic) harness
                    match store.list_harnesses().await {
                        Ok(harnesses) => {
                            match harnesses
                                .iter()
                                .find(|h| h.is_built_in && h.name == "Generic")
                            {
                                Some(h) => h.id,
                                None => {
                                    return ToolExecutionResult::tool_error(
                                        "No harness_id provided and no default Generic harness found. Please specify a harness_id.",
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            return ToolExecutionResult::tool_error(format!(
                                "No harness_id provided and failed to resolve default harness: {e}"
                            ));
                        }
                    }
                };
                let agent_id = get_str(&arguments, "agent_id")
                    .and_then(|s| s.parse::<crate::typed_id::AgentId>().ok());
                let title = get_str(&arguments, "title");
                let locale = get_str(&arguments, "locale");
                match store
                    .create_session(harness_id, agent_id, title, locale, None, None)
                    .await
                {
                    Ok(s) => ToolExecutionResult::success(json!({
                        "id": s.id.to_string(),
                        "organization_id": s.organization_id,
                        "title": s.title,
                        "locale": s.locale,
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
                "Unknown operation: {operation}. Valid: create, delete"
            )),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Tool: session_send_message
// =============================================================================

pub struct SessionSendMessageTool;

#[async_trait]
impl Tool for SessionSendMessageTool {
    fn name(&self) -> &str {
        "session_send_message"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Send Message")
    }

    fn description(&self) -> &str {
        "Send a user message to a session, triggering a turn."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Target session ID"
                },
                "content": {
                    "type": "string",
                    "description": "Message content"
                }
            },
            "required": ["session_id", "content"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "session_send_message requires context. This tool must be executed with session context.",
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
        let content = match require_str(&arguments, "content") {
            Ok(s) => s,
            Err(e) => return e,
        };

        let base_url = store.base_url();

        match store.send_message(session_id, content).await {
            Ok(()) => ToolExecutionResult::success(json!({
                "session_id": session_id_str,
                "message": "Message sent successfully. Use session_read_response to wait for the agent response.",
                "ui_link": format!("{}/sessions/{}/chat", base_url, session_id),
            })),
            Err(e) => ToolExecutionResult::tool_error(format!("Failed to send message: {e}")),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Tool: session_read_messages
// =============================================================================

pub struct SessionReadMessagesTool;

#[async_trait]
impl Tool for SessionReadMessagesTool {
    fn name(&self) -> &str {
        "session_read_messages"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Read Messages")
    }

    fn description(&self) -> &str {
        "Read messages from a session."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Target session ID"
                },
                "limit": {
                    "type": "integer",
                    "description": "Optional max messages (default: 10)"
                }
            },
            "required": ["session_id"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "session_read_messages requires context. This tool must be executed with session context.",
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

        let limit = arguments
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);

        let base_url = store.base_url();

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
            Err(e) => ToolExecutionResult::tool_error(format!("Failed to get messages: {e}")),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Tool: session_read_response
// =============================================================================

pub struct SessionReadResponseTool;

#[async_trait]
impl Tool for SessionReadResponseTool {
    fn name(&self) -> &str {
        "session_read_response"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Read Response")
    }

    fn description(&self) -> &str {
        "Wait for session to finish processing and return the response. Set timeout_secs to 0 to check status without waiting."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Target session ID"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Optional timeout (default: 120). Set to 0 to check status without waiting."
                }
            },
            "required": ["session_id"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "session_read_response requires context. This tool must be executed with session context.",
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

        let timeout_secs = arguments.get("timeout_secs").and_then(|v| v.as_u64());
        let base_url = store.base_url();

        match store.wait_for_idle(session_id, timeout_secs).await {
            Ok(status) => ToolExecutionResult::success(json!({
                "session_id": session_id_str,
                "status": status,
                "ui_link": format!("{}/sessions/{}/chat", base_url, session_id),
            })),
            Err(e) => ToolExecutionResult::tool_error(format!("Failed waiting for response: {e}")),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// =============================================================================
// Tool: read_capabilities
// =============================================================================

pub struct ReadCapabilitiesTool;

#[async_trait]
impl Tool for ReadCapabilitiesTool {
    fn name(&self) -> &str {
        "read_capabilities"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Read Capabilities")
    }

    fn description(&self) -> &str {
        "Discover available capabilities (built-in, MCP servers, and skills). Use this to find capability IDs before creating or updating agents and harnesses."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Optional capability ID to get a single capability"
                },
                "search": {
                    "type": "string",
                    "description": "Optional search query to filter capabilities by name, description, category, or ID (case-insensitive)"
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
        ToolExecutionResult::tool_error(
            "read_capabilities requires context. This tool must be executed with session context.",
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

        let id_filter = get_str(&arguments, "id");
        let search = get_str(&arguments, "search");

        // If id is given, use it as the search filter to find the specific capability
        let effective_search = id_filter.or(search);

        match store.list_capabilities(effective_search).await {
            Ok(capabilities) => {
                let items: Vec<Value> = capabilities
                    .iter()
                    .map(|c| {
                        let mut item = json!({
                            "id": c.id.as_str(),
                            "name": c.name,
                            "description": c.description,
                            "status": c.status.to_string(),
                        });
                        if let Some(cat) = &c.category {
                            item["category"] = json!(cat);
                        }
                        if c.is_mcp {
                            item["type"] = json!("mcp_server");
                        } else if c.is_skill {
                            item["type"] = json!("skill");
                        } else {
                            item["type"] = json!("builtin");
                        }
                        if !c.tool_definitions.is_empty() {
                            item["tool_count"] = json!(c.tool_definitions.len());
                            item["tools"] = json!(
                                c.tool_definitions
                                    .iter()
                                    .map(|t| t.name())
                                    .collect::<Vec<_>>()
                            );
                        }
                        if !c.dependencies.is_empty() {
                            item["dependencies"] = json!(c.dependencies);
                        }
                        item
                    })
                    .collect();

                // When id is provided, return exact match as single item
                if let Some(target_id) = id_filter {
                    if let Some(exact) = items.iter().find(|i| i["id"].as_str() == Some(target_id))
                    {
                        return ToolExecutionResult::success(exact.clone());
                    }
                    return ToolExecutionResult::tool_error(format!(
                        "Capability not found: {target_id}"
                    ));
                }

                let count = items.len();
                ToolExecutionResult::success(json!({
                    "capabilities": items,
                    "count": count,
                    "hint": "Use capability IDs when creating or updating agents and harnesses via manage_agents or manage_harnesses (capabilities parameter)."
                }))
            }
            Err(e) => ToolExecutionResult::tool_error(format!("Failed to list capabilities: {e}")),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform_store::PlatformStore;
    use crate::platform_store::tests::MockPlatformStore;
    use crate::typed_id::{AgentId, HarnessId, SessionId};
    use std::sync::Arc;

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
    fn capability_provides_ten_tools() {
        let cap = PlatformManagementCapability;
        let tools = cap.tools();
        assert_eq!(tools.len(), 10);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"read_capabilities"));
        assert!(names.contains(&"read_harnesses"));
        assert!(names.contains(&"manage_harnesses"));
        assert!(names.contains(&"read_agents"));
        assert!(names.contains(&"manage_agents"));
        assert!(names.contains(&"read_sessions"));
        assert!(names.contains(&"manage_sessions"));
        assert!(names.contains(&"session_send_message"));
        assert!(names.contains(&"session_read_messages"));
        assert!(names.contains(&"session_read_response"));
    }

    // =========================================================================
    // ReadHarnessesTool tests
    // =========================================================================

    #[tokio::test]
    async fn read_harnesses_list_returns_harnesses_with_ui_link() {
        let ctx = mock_context();
        let tool = ReadHarnessesTool;
        let result = tool.execute_with_context(json!({}), &ctx).await;
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
    async fn read_harnesses_get_by_id_returns_full_detail() {
        let ctx = mock_context();
        let tool = ReadHarnessesTool;
        let result = tool
            .execute_with_context(json!({"id": HarnessId::new().to_string()}), &ctx)
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["name"], "test-harness");
                assert_eq!(v["display_name"], "Test Harness");
                assert!(v["system_prompt"].as_str().is_some());
                assert!(v["ui_link"].as_str().unwrap().contains("/harnesses/"));
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_harnesses_invalid_id_returns_error() {
        let ctx = mock_context();
        let tool = ReadHarnessesTool;
        let result = tool.execute_with_context(json!({"id": "bad"}), &ctx).await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Invalid harness id")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    // =========================================================================
    // ManageHarnessesTool tests
    // =========================================================================

    #[tokio::test]
    async fn harness_create_returns_new_harness() {
        let ctx = mock_context();
        let tool = ManageHarnessesTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "create", "name": "my-harness", "display_name": "My Harness", "system_prompt": "Be fun!"}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["name"], "my-harness");
                assert_eq!(v["display_name"], "My Harness");
                assert!(
                    v["ui_link"]
                        .as_str()
                        .unwrap()
                        .starts_with("http://localhost:9300/harnesses/")
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
    async fn harness_missing_required_param_returns_error() {
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
    // ReadAgentsTool tests
    // =========================================================================

    #[tokio::test]
    async fn read_agents_list_returns_agents() {
        let ctx = mock_context();
        let tool = ReadAgentsTool;
        let result = tool.execute_with_context(json!({}), &ctx).await;
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
    async fn read_agents_get_by_id_succeeds() {
        let ctx = mock_context();
        let tool = ReadAgentsTool;
        let result = tool
            .execute_with_context(json!({"id": AgentId::new().to_string()}), &ctx)
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
    async fn read_agents_invalid_id_returns_error() {
        let ctx = mock_context();
        let tool = ReadAgentsTool;
        let result = tool
            .execute_with_context(json!({"id": "not-valid"}), &ctx)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Invalid agent id")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    // =========================================================================
    // ManageAgentsTool tests
    // =========================================================================

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

    // =========================================================================
    // ReadSessionsTool tests
    // =========================================================================

    #[tokio::test]
    async fn read_sessions_list_returns_sessions() {
        let ctx = mock_context();
        let tool = ReadSessionsTool;
        let result = tool.execute_with_context(json!({}), &ctx).await;
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
    async fn read_sessions_get_by_id_succeeds() {
        let ctx = mock_context();
        let tool = ReadSessionsTool;
        let result = tool
            .execute_with_context(json!({"id": SessionId::new().to_string()}), &ctx)
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
    async fn read_sessions_invalid_id_returns_error() {
        let ctx = mock_context();
        let tool = ReadSessionsTool;
        let result = tool.execute_with_context(json!({"id": "nope"}), &ctx).await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Invalid session id")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    // =========================================================================
    // ManageSessionsTool tests
    // =========================================================================

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
    async fn session_create_missing_harness_id_falls_back_to_generic() {
        let ctx = mock_context();
        let tool = ManageSessionsTool;
        // Mock store has no built-in Generic harness, so fallback should error
        let result = tool
            .execute_with_context(json!({"operation": "create"}), &ctx)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("no default Generic harness found"))
            }
            other => panic!("expected tool error for missing Generic harness, got: {other:?}"),
        }
    }

    // =========================================================================
    // SessionSendMessageTool tests
    // =========================================================================

    #[tokio::test]
    async fn send_message_succeeds() {
        let ctx = mock_context();
        let tool = SessionSendMessageTool;
        let result = tool
            .execute_with_context(
                json!({"session_id": SessionId::new().to_string(), "content": "Hi!"}),
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
    async fn send_message_missing_content_returns_error() {
        let ctx = mock_context();
        let tool = SessionSendMessageTool;
        let result = tool
            .execute_with_context(json!({"session_id": SessionId::new().to_string()}), &ctx)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Missing required")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn send_message_invalid_session_id_returns_error() {
        let ctx = mock_context();
        let tool = SessionSendMessageTool;
        let result = tool
            .execute_with_context(json!({"session_id": "bad-id", "content": "Hi!"}), &ctx)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Invalid session_id")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    // =========================================================================
    // SessionReadMessagesTool tests
    // =========================================================================

    #[tokio::test]
    async fn read_messages_returns_messages() {
        let ctx = mock_context();
        let tool = SessionReadMessagesTool;
        let result = tool
            .execute_with_context(
                json!({"session_id": SessionId::new().to_string(), "limit": 5}),
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
    async fn read_messages_invalid_session_id_returns_error() {
        let ctx = mock_context();
        let tool = SessionReadMessagesTool;
        let result = tool
            .execute_with_context(json!({"session_id": "bad-id"}), &ctx)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Invalid session_id")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_messages_missing_session_id_returns_error() {
        let ctx = mock_context();
        let tool = SessionReadMessagesTool;
        let result = tool.execute_with_context(json!({}), &ctx).await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Missing required")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    // =========================================================================
    // SessionReadResponseTool tests
    // =========================================================================

    #[tokio::test]
    async fn read_response_succeeds() {
        let ctx = mock_context();
        let tool = SessionReadResponseTool;
        let result = tool
            .execute_with_context(json!({"session_id": SessionId::new().to_string()}), &ctx)
            .await;
        match result {
            ToolExecutionResult::Success(v) => assert_eq!(v["status"], "idle"),
            other => panic!("expected success, got: {other:?}"),
        }
    }

    // =========================================================================
    // Context and error tests
    // =========================================================================

    #[tokio::test]
    async fn tool_without_context_returns_error() {
        let tool = ManageHarnessesTool;
        let result = tool.execute(json!({"operation": "create"})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("requires context")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn tool_without_platform_store_returns_error() {
        let ctx = ToolContext::new(SessionId::new());
        let tool = ReadHarnessesTool;
        let result = tool.execute_with_context(json!({}), &ctx).await;
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
    async fn all_tools_require_context() {
        assert!(ReadCapabilitiesTool.requires_context());
        assert!(ReadHarnessesTool.requires_context());
        assert!(ManageHarnessesTool.requires_context());
        assert!(ReadAgentsTool.requires_context());
        assert!(ManageAgentsTool.requires_context());
        assert!(ReadSessionsTool.requires_context());
        assert!(ManageSessionsTool.requires_context());
        assert!(SessionSendMessageTool.requires_context());
        assert!(SessionReadMessagesTool.requires_context());
        assert!(SessionReadResponseTool.requires_context());
    }

    #[tokio::test]
    async fn all_tools_without_context_return_error() {
        // execute() (no context) should fail for all tools
        for tool_name in [
            "read_capabilities",
            "read_harnesses",
            "manage_harnesses",
            "read_agents",
            "manage_agents",
            "read_sessions",
            "manage_sessions",
            "session_send_message",
            "session_read_messages",
            "session_read_response",
        ] {
            let result = match tool_name {
                "read_capabilities" => ReadCapabilitiesTool.execute(json!({})).await,
                "read_harnesses" => ReadHarnessesTool.execute(json!({})).await,
                "manage_harnesses" => {
                    ManageHarnessesTool
                        .execute(json!({"operation": "create"}))
                        .await
                }
                "read_agents" => ReadAgentsTool.execute(json!({})).await,
                "manage_agents" => {
                    ManageAgentsTool
                        .execute(json!({"operation": "create"}))
                        .await
                }
                "read_sessions" => ReadSessionsTool.execute(json!({})).await,
                "manage_sessions" => {
                    ManageSessionsTool
                        .execute(json!({"operation": "create"}))
                        .await
                }
                "session_send_message" => {
                    SessionSendMessageTool
                        .execute(json!({"session_id": "x", "content": "hi"}))
                        .await
                }
                "session_read_messages" => {
                    SessionReadMessagesTool
                        .execute(json!({"session_id": "x"}))
                        .await
                }
                "session_read_response" => {
                    SessionReadResponseTool
                        .execute(json!({"session_id": "x"}))
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

    // =========================================================================
    // ReadCapabilitiesTool tests
    // =========================================================================

    #[tokio::test]
    async fn read_capabilities_returns_all() {
        let ctx = mock_context();
        let tool = ReadCapabilitiesTool;
        let result = tool.execute_with_context(json!({}), &ctx).await;
        match result {
            ToolExecutionResult::Success(v) => {
                let count = v["count"].as_u64().unwrap();
                assert!(count > 0, "should return at least one capability");
                let caps = v["capabilities"].as_array().unwrap();
                for cap in caps {
                    assert!(cap["id"].is_string());
                    assert!(cap["name"].is_string());
                    assert!(cap["type"].is_string());
                }
                assert!(v["hint"].as_str().unwrap().contains("capability IDs"));
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_capabilities_search_filters_results() {
        let ctx = mock_context();
        let tool = ReadCapabilitiesTool;
        let result = tool
            .execute_with_context(json!({"search": "current_time"}), &ctx)
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                let count = v["count"].as_u64().unwrap();
                assert!(count >= 1, "should find at least current_time");
                let caps = v["capabilities"].as_array().unwrap();
                assert!(
                    caps.iter()
                        .any(|c| c["id"].as_str().unwrap() == "current_time"),
                    "should contain current_time"
                );
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_capabilities_search_no_match() {
        let ctx = mock_context();
        let tool = ReadCapabilitiesTool;
        let result = tool
            .execute_with_context(json!({"search": "zzz_nonexistent_zzz"}), &ctx)
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["count"], 0);
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_capabilities_empty_id_returns_all() {
        let ctx = mock_context();
        let tool = ReadCapabilitiesTool;
        // LLMs sometimes send empty strings for optional params — must not crash
        let result = tool
            .execute_with_context(json!({"id": "", "search": ""}), &ctx)
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                let count = v["count"].as_u64().unwrap();
                assert!(count > 0, "empty id/search should return all capabilities");
            }
            other => panic!("expected success with all capabilities, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_capabilities_empty_id_only_returns_all() {
        let ctx = mock_context();
        let tool = ReadCapabilitiesTool;
        let result = tool.execute_with_context(json!({"id": ""}), &ctx).await;
        match result {
            ToolExecutionResult::Success(v) => {
                let count = v["count"].as_u64().unwrap();
                assert!(count > 0, "empty id should return all capabilities");
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[test]
    fn capability_has_system_prompt_addition() {
        let cap = PlatformManagementCapability;
        let prompt = cap.system_prompt_addition().expect("should have prompt");
        assert!(prompt.contains("read_capabilities"));
        assert!(prompt.contains("Capabilities"));
    }
}
