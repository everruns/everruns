// Platform Management capability
// THREAT[TM-AGENT-017]: Agents with this capability can manage org-wide entities
//
// Decision: Read/write split — read tools (read_*) return single item by ID or filtered list;
//           write tools (manage_*) perform mutations. Session I/O split into three single-purpose tools.
// Decision: All results include UI links via PlatformStore::base_url()
// Decision: get_messages defaults to last 10; session_read_response defaults to 120s timeout
// Decision: Platform docs (docs/) are optionally embedded at compile time and mounted
//           as a virtual readonly filesystem at /docs. This gives the platform chat agent
//           access to Everruns documentation without DB writes per session while allowing
//           the public core crate to build without repo-root files.

use crate::app::{App, AppChannel, ChannelType};
use async_trait::async_trait;
use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, MountPoint, is_declarative_capability,
};
#[cfg(all(feature = "embedded-platform-docs", everruns_has_workspace_docs))]
use everruns_core::capability_types::{MountAccess, MountSource, VirtualFileTree};
use everruns_core::tool_types::ToolHints;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;
#[cfg(all(feature = "embedded-platform-docs", everruns_has_workspace_docs))]
use include_dir::{Dir, include_dir};
use serde_json::{Value, json};
#[cfg(all(feature = "embedded-platform-docs", everruns_has_workspace_docs))]
use std::sync::Arc;

const SESSION_READ_MESSAGES_DEFAULT_LIMIT: usize = 10;
const SESSION_READ_MESSAGES_MAX_LIMIT: usize = 50;
const SESSION_READ_MESSAGES_DEFAULT_CONTENT_LIMIT: usize = 12_000;
const SESSION_READ_MESSAGES_MAX_CONTENT_LIMIT: usize = 50_000;

// =============================================================================
// Capability
// =============================================================================

// =============================================================================
// Embedded platform docs (virtual mount)
// =============================================================================

/// Docs directory embedded at compile time from the repo root `docs/`.
#[cfg(all(feature = "embedded-platform-docs", everruns_has_workspace_docs))]
static DOCS_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/../../docs");

#[cfg(all(feature = "embedded-platform-docs", everruns_has_workspace_docs))]
fn dir_to_tree(dir: &Dir, base: &str) -> VirtualFileTree {
    let mut tree = VirtualFileTree::new();
    tree.insert_directory(base);
    populate_tree(&mut tree, dir, base);
    tree
}

#[cfg(all(feature = "embedded-platform-docs", everruns_has_workspace_docs))]
fn populate_tree(tree: &mut VirtualFileTree, dir: &Dir, prefix: &str) {
    for file in dir.files() {
        let name = file
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        // Only include markdown content (skip images, JSON, etc.)
        let ext = file
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !matches!(ext, "md" | "mdx") {
            continue;
        }
        let path = format!("{prefix}/{name}");
        let content = std::str::from_utf8(file.contents()).unwrap_or("");
        tree.insert_text(&path, content);
    }
    for subdir in dir.dirs() {
        let name = subdir
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let path = format!("{prefix}/{name}");
        tree.insert_directory(&path);
        populate_tree(tree, subdir, &path);
    }
}

/// Lazily-initialized shared tree (built once on first access).
#[cfg(all(feature = "embedded-platform-docs", everruns_has_workspace_docs))]
pub(super) fn docs_tree() -> Arc<VirtualFileTree> {
    use std::sync::OnceLock;
    static TREE: OnceLock<Arc<VirtualFileTree>> = OnceLock::new();
    TREE.get_or_init(|| Arc::new(dir_to_tree(&DOCS_DIR, "/docs")))
        .clone()
}

#[cfg(all(feature = "embedded-platform-docs", everruns_has_workspace_docs))]
const SYSTEM_PROMPT: &str = r#"Capabilities extend agent/harness functionality. Three types: built-in, MCP servers, and skills. Use `read_capabilities` to discover IDs before creating agents/harnesses. All results include UI links.
<platform-docs>
Platform documentation is available at /workspace/docs in the session filesystem.
Use `read_file`, `list_directory`, or `grep` to browse and search it.
Bash commands like `cat /workspace/docs/...`, `ls /workspace/docs/`, and
`grep -r "pattern" /workspace/docs/` also work.

Key sections:
- /workspace/docs/getting-started/ — Introduction, concepts, architecture, Docker setup
- /workspace/docs/features/ — SDK, CLI, UI, events, harnesses, capabilities, apps, skills
- /workspace/docs/capabilities/ — Per-capability reference (file-system, bashkit-shell, web-fetch, etc.)
- /workspace/docs/integrations/ — External integrations (Slack, Daytona, Browserless, etc.)
- /workspace/docs/advanced/ — Budgets, compaction, embedding, network access, request signing
- /workspace/docs/sre/ — Environment variables, admin container, runbooks

When the user asks about Everruns features, configuration, or how things work,
consult these docs before answering.
</platform-docs>"#;

#[cfg(not(all(feature = "embedded-platform-docs", everruns_has_workspace_docs)))]
const SYSTEM_PROMPT: &str = "Capabilities extend agent/harness functionality. Three types: built-in, MCP servers, and skills. Use `read_capabilities` to discover IDs before creating agents/harnesses. All results include UI links.";

pub const PLATFORM_MANAGEMENT_CAPABILITY_ID: &str = "platform_management";

pub struct PlatformManagementCapability;

impl Capability for PlatformManagementCapability {
    fn id(&self) -> &str {
        PLATFORM_MANAGEMENT_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Platform Management"
    }

    fn description(&self) -> &str {
        "Tools to manage harnesses, agents, apps, channels, and sessions. Create, list, update, delete entities and interact with sessions programmatically."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Керування платформою",
            "Інструменти для керування harness-ами, агентами, застосунками, каналами та сесіями. Створюйте, переглядайте, оновлюйте й видаляйте сутності та взаємодійте із сесіями програмно.",
        )]
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
        Some(SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(ReadCapabilitiesTool),
            Box::new(ReadHarnessesTool),
            Box::new(ManageHarnessesTool),
            Box::new(ReadAgentsTool),
            Box::new(ManageAgentsTool),
            Box::new(ReadAppsTool),
            Box::new(ManageAppsTool),
            Box::new(ManageAppChannelsTool),
            Box::new(ReadSessionsTool),
            Box::new(SessionContextReportTool),
            Box::new(ManageSessionsTool),
            Box::new(SessionSendMessageTool),
            Box::new(SessionReadMessagesTool),
            Box::new(SessionReadResponseTool),
        ]
    }

    fn mounts(&self) -> Vec<MountPoint> {
        #[cfg(all(feature = "embedded-platform-docs", everruns_has_workspace_docs))]
        {
            vec![MountPoint::new(
                "/docs",
                MountAccess::ReadOnly,
                MountSource::Virtual { tree: docs_tree() },
                self.id(),
            )]
        }
        #[cfg(not(all(feature = "embedded-platform-docs", everruns_has_workspace_docs)))]
        {
            Vec::new()
        }
    }

    fn dependencies(&self) -> Vec<&'static str> {
        #[cfg(all(feature = "embedded-platform-docs", everruns_has_workspace_docs))]
        {
            vec!["session_file_system"]
        }
        #[cfg(not(all(feature = "embedded-platform-docs", everruns_has_workspace_docs)))]
        {
            Vec::new()
        }
    }
}

// =============================================================================
// Helper: argument extraction / typed-ID parsing / store lookup
// =============================================================================
//
// Canonical scaffolding lives in `super::util`. Tool logic lives in `_impl`
// functions returning `Result<ToolExecutionResult, ToolExecutionResult>` so the
// helpers compose with `?`; each trait `execute_with_context` calls the `_impl`
// via `.unwrap_or_else(|e| e)`.

use super::util::{get_platform_store, get_str, parse_id, require_id, require_str};

fn parse_bounded_usize_arg(
    args: &Value,
    key: &str,
    default: usize,
    max: usize,
) -> Result<usize, ToolExecutionResult> {
    match args.get(key).and_then(|v| v.as_u64()) {
        Some(0) => Err(ToolExecutionResult::tool_error(format!(
            "{key} must be greater than 0"
        ))),
        Some(value) => Ok((value as usize).min(max)),
        None => Ok(default),
    }
}

fn parse_channel_type(value: &str, field: &str) -> Result<ChannelType, ToolExecutionResult> {
    serde_json::from_value(Value::String(value.to_string()))
        .map_err(|_| ToolExecutionResult::tool_error(format!("Invalid {field}: {value}")))
}

fn truncate_content_chars(content: &str, limit: usize) -> (String, bool, usize, usize) {
    let mut end_byte = content.len();
    let mut returned_chars = 0;
    let mut total_chars = 0;

    for (idx, (byte_idx, _)) in content.char_indices().enumerate() {
        total_chars = idx + 1;
        if idx == limit {
            end_byte = byte_idx;
        }
        if idx < limit {
            returned_chars = idx + 1;
        }
    }

    let truncated = total_chars > limit;
    if !truncated {
        return (content.to_string(), false, total_chars, total_chars);
    }

    (
        content[..end_byte].to_string(),
        true,
        total_chars,
        returned_chars,
    )
}

fn channel_json(channel: &AppChannel, include_config: bool) -> Value {
    let mut json = json!({
        "id": channel.public_id.to_string(),
        "channel_type": channel.channel_type.to_string(),
        "enabled": channel.enabled,
        "created_at": channel.created_at.to_rfc3339(),
        "updated_at": channel.updated_at.to_rfc3339(),
    });
    if include_config {
        json["channel_config"] = channel.channel_config.clone();
    }
    json
}

fn app_json(app: &App, base_url: &str, include_channel_config: bool) -> Value {
    json!({
        "id": app.public_id.to_string(),
        "name": app.name,
        "description": app.description,
        "status": app.status.to_string(),
        "harness_id": app.harness_id.to_string(),
        "agent_id": app.agent_id.as_ref().map(|id| id.to_string()),
        "agent_identity_id": app.agent_identity_id.as_ref().map(|id| id.to_string()),
        "published_at": app.published_at.map(|value| value.to_rfc3339()),
        "created_at": app.created_at.to_rfc3339(),
        "updated_at": app.updated_at.to_rfc3339(),
        "channel_count": app.channels.len(),
        "channels": app
            .channels
            .iter()
            .map(|channel| channel_json(channel, include_channel_config))
            .collect::<Vec<_>>(),
        "ui_link": format!("{}/apps/{}", base_url, app.public_id),
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
        read_harnesses_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

async fn read_harnesses_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let store = get_platform_store(context)?;
    let base_url = store.base_url();

    if let Some(id_str) = get_str(&arguments, "id") {
        let id: everruns_core::typed_id::HarnessId = parse_id(id_str, "harness id")?;
        return Ok(match store.get_harness(id).await {
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
        });
    }

    Ok(match store.list_harnesses().await {
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
    })
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
                    "description": "Optional base system prompt for the harness. Omit to contribute no base prompt — the effective prompt then comes from the parent harness, agent, session, and capabilities."
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
        ToolHints::default().with_narration_noun("harness")
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
        manage_harnesses_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

async fn manage_harnesses_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let store = get_platform_store(context)?;
    let operation = require_str(&arguments, "operation")?;
    let base_url = store.base_url();

    Ok(match operation {
        "create" => {
            let name = require_str(&arguments, "name")?;
            let display_name = require_str(&arguments, "display_name")?;
            // Optional: when omitted the harness contributes no base prompt.
            let system_prompt = get_str(&arguments, "system_prompt");
            let description = get_str(&arguments, "description");
            let parent_harness_id = match arguments.get("parent_harness_id") {
                Some(Value::String(id_str)) => Some(
                    parse_id::<everruns_core::typed_id::HarnessId>(id_str, "parent_harness_id")?,
                ),
                Some(Value::Null) | None => None,
                Some(_) => {
                    return Ok(ToolExecutionResult::tool_error(
                        "parent_harness_id must be a harness ID string or null",
                    ));
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
                    Some(display_name),
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
                Err(e) => ToolExecutionResult::tool_error(format!("Failed to create harness: {e}")),
            }
        }

        "update" => {
            let id_str = require_str(&arguments, "harness_id")?;
            let id: everruns_core::typed_id::HarnessId = parse_id(id_str, "harness_id")?;
            let name = get_str(&arguments, "name");
            let display_name = get_str(&arguments, "display_name");
            let description = get_str(&arguments, "description");
            let system_prompt = get_str(&arguments, "system_prompt");
            let parent_harness_id = match arguments.get("parent_harness_id") {
                Some(Value::String(id_str)) => Some(Some(parse_id::<
                    everruns_core::typed_id::HarnessId,
                >(
                    id_str, "parent_harness_id"
                )?)),
                Some(Value::Null) => Some(None),
                None => None,
                Some(_) => {
                    return Ok(ToolExecutionResult::tool_error(
                        "parent_harness_id must be a harness ID string or null",
                    ));
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
                Err(e) => ToolExecutionResult::tool_error(format!("Failed to update harness: {e}")),
            }
        }

        "delete" => {
            let id_str = require_str(&arguments, "harness_id")?;
            let id: everruns_core::typed_id::HarnessId = parse_id(id_str, "harness_id")?;
            match store.delete_harness(id).await {
                Ok(()) => ToolExecutionResult::success(json!({
                    "harness_id": id_str,
                    "ui_link": format!("{}/harnesses/{}", base_url, id_str),
                    "message": "Harness archived successfully"
                })),
                Err(e) => ToolExecutionResult::tool_error(format!("Failed to delete harness: {e}")),
            }
        }

        "copy" => {
            let id_str = require_str(&arguments, "harness_id")?;
            let id: everruns_core::typed_id::HarnessId = parse_id(id_str, "harness_id")?;
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
                Err(e) => ToolExecutionResult::tool_error(format!("Failed to copy harness: {e}")),
            }
        }

        _ => ToolExecutionResult::tool_error(format!(
            "Unknown operation: {operation}. Valid: create, update, delete, copy"
        )),
    })
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
        read_agents_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

async fn read_agents_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let store = get_platform_store(context)?;
    let base_url = store.base_url();

    if let Some(id_str) = get_str(&arguments, "id") {
        let id: everruns_core::typed_id::AgentId = parse_id(id_str, "agent id")?;
        return Ok(match store.get_agent_by_id(id).await {
            Ok(Some(a)) => ToolExecutionResult::success(json!({
                "id": a.public_id.to_string(),
                "name": a.name,
                "display_name": a.display_name,
                "description": a.description,
                "system_prompt": a.system_prompt,
                "status": format!("{:?}", a.status),
                "capabilities": a.capabilities.iter().map(|c| c.capability_id().to_string()).collect::<Vec<_>>(),
                "tags": a.tags,
                "ui_link": format!("{}/agents/{}", base_url, a.public_id),
            })),
            Ok(None) => ToolExecutionResult::tool_error(format!("Agent not found: {id_str}")),
            Err(e) => ToolExecutionResult::tool_error(format!("Failed to get agent: {e}")),
        });
    }

    Ok(match store.list_agents().await {
        Ok(agents) => {
            let items: Vec<Value> = agents
                .iter()
                .map(|a| {
                    json!({
                        "id": a.public_id.to_string(),
                        "name": a.name,
                        "display_name": a.display_name,
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
    })
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
                    "description": "Addressable agent name (required for create). Lowercase letters, numbers, and hyphens only (e.g. 'customer-support')."
                },
                "display_name": {
                    "type": "string",
                    "description": "Human-readable display name shown in UI (e.g. 'Customer Support Agent'). Falls back to name when absent."
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
        ToolHints::default().with_narration_noun("agent")
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
        manage_agents_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

async fn manage_agents_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let store = get_platform_store(context)?;
    let operation = require_str(&arguments, "operation")?;
    let base_url = store.base_url();

    Ok(match operation {
        "create" => {
            let name = require_str(&arguments, "name")?;
            if let Err(msg) = crate::agent::validate_addressable_name(name) {
                return Ok(ToolExecutionResult::tool_error(format!(
                    "Invalid agent name: {msg}"
                )));
            }
            let display_name = get_str(&arguments, "display_name");
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
                .create_agent(
                    name,
                    display_name,
                    description,
                    system_prompt,
                    &capabilities,
                )
                .await
            {
                Ok(a) => ToolExecutionResult::success(json!({
                    "id": a.public_id.to_string(),
                    "name": a.name,
                    "display_name": a.display_name,
                    "description": a.description,
                    "status": format!("{:?}", a.status),
                    "ui_link": format!("{}/agents/{}", base_url, a.public_id),
                    "message": "Agent created successfully"
                })),
                Err(e) => ToolExecutionResult::tool_error(format!("Failed to create agent: {e}")),
            }
        }

        "update" => {
            let id_str = require_str(&arguments, "agent_id")?;
            let id: everruns_core::typed_id::AgentId = parse_id(id_str, "agent_id")?;
            let name = get_str(&arguments, "name");
            if let Some(n) = name
                && let Err(msg) = crate::agent::validate_addressable_name(n)
            {
                return Ok(ToolExecutionResult::tool_error(format!(
                    "Invalid agent name: {msg}"
                )));
            }
            let display_name = get_str(&arguments, "display_name");
            let description = get_str(&arguments, "description");
            let system_prompt = get_str(&arguments, "system_prompt");
            match store
                .update_agent(id, name, display_name, description, system_prompt)
                .await
            {
                Ok(a) => ToolExecutionResult::success(json!({
                    "id": a.public_id.to_string(),
                    "name": a.name,
                    "display_name": a.display_name,
                    "description": a.description,
                    "status": format!("{:?}", a.status),
                    "ui_link": format!("{}/agents/{}", base_url, a.public_id),
                    "message": "Agent updated successfully"
                })),
                Err(e) => ToolExecutionResult::tool_error(format!("Failed to update agent: {e}")),
            }
        }

        "delete" => {
            let id_str = require_str(&arguments, "agent_id")?;
            let id: everruns_core::typed_id::AgentId = parse_id(id_str, "agent_id")?;
            match store.delete_agent(id).await {
                Ok(()) => ToolExecutionResult::success(json!({
                    "agent_id": id_str,
                    "ui_link": format!("{}/agents/{}", base_url, id_str),
                    "message": "Agent archived successfully"
                })),
                Err(e) => ToolExecutionResult::tool_error(format!("Failed to delete agent: {e}")),
            }
        }

        _ => ToolExecutionResult::tool_error(format!(
            "Unknown operation: {operation}. Valid: create, update, delete"
        )),
    })
}

// =============================================================================
// Tool: read_apps (read-only: get by ID or list/filter)
// =============================================================================

pub struct ReadAppsTool;

#[async_trait]
impl Tool for ReadAppsTool {
    fn name(&self) -> &str {
        "read_apps"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Read Apps")
    }

    fn description(&self) -> &str {
        "Read apps by ID or list/filter. When id is provided returns full app detail including channels."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Optional app ID to get a single app with channel details"
                },
                "search": {
                    "type": "string",
                    "description": "Optional case-insensitive search by app name or description"
                },
                "include_archived": {
                    "type": "boolean",
                    "description": "Include archived apps in list results (default: false)"
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
            "read_apps requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        read_apps_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

async fn read_apps_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let store = get_platform_store(context)?;
    let base_url = store.base_url();

    if let Some(id_str) = get_str(&arguments, "id") {
        let id: everruns_core::typed_id::AppId = parse_id(id_str, "app id")?;
        return Ok(match store.get_app(id).await {
            Ok(Some(app)) => ToolExecutionResult::success(app_json(&app, base_url, true)),
            Ok(None) => ToolExecutionResult::tool_error(format!("App not found: {id_str}")),
            Err(e) => ToolExecutionResult::tool_error(format!("Failed to get app: {e}")),
        });
    }

    let search = get_str(&arguments, "search");
    let include_archived = arguments
        .get("include_archived")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    Ok(match store.list_apps(search, include_archived).await {
        Ok(apps) => {
            let items = apps
                .iter()
                .map(|app| app_json(app, base_url, false))
                .collect::<Vec<_>>();
            ToolExecutionResult::success(json!({"apps": items, "count": items.len()}))
        }
        Err(e) => ToolExecutionResult::tool_error(format!("Failed to list apps: {e}")),
    })
}

// =============================================================================
// Tool: manage_apps (mutations: create, update, delete, destroy, publish, unpublish)
// =============================================================================

pub struct ManageAppsTool;

#[async_trait]
impl Tool for ManageAppsTool {
    fn name(&self) -> &str {
        "manage_apps"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Manage Apps")
    }

    fn description(&self) -> &str {
        "App mutations: create, update, delete (archive), destroy, publish, unpublish."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["create", "update", "delete", "destroy", "publish", "unpublish"],
                    "description": "The mutation to perform"
                },
                "app_id": {
                    "type": "string",
                    "description": "App ID (required for update/delete/destroy/publish/unpublish)"
                },
                "name": {
                    "type": "string",
                    "description": "App name (required for create)"
                },
                "description": {
                    "type": "string",
                    "description": "App description (optional)"
                },
                "harness_id": {
                    "type": "string",
                    "description": "Harness ID (required for create)"
                },
                "agent_id": {
                    "type": "string",
                    "description": "Optional agent ID"
                },
                "agent_identity_id": {
                    "type": ["string", "null"],
                    "description": "Optional agent identity ID. Pass null on update to clear it."
                },
                "channel_type": {
                    "type": "string",
                    "enum": ["slack", "ag_ui", "schedule", "webhook", "a2a", "fcp"],
                    "description": "Optional initial channel type for create"
                },
                "channel_config": {
                    "type": "object",
                    "description": "Optional initial channel configuration for create"
                }
            },
            "required": ["operation"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_narration_noun("app")
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "manage_apps requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        manage_apps_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

async fn manage_apps_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let store = get_platform_store(context)?;
    let operation = require_str(&arguments, "operation")?;
    let base_url = store.base_url();

    Ok(match operation {
        "create" => {
            let name = require_str(&arguments, "name")?;
            let harness_id_str = require_str(&arguments, "harness_id")?;
            let harness_id: everruns_core::typed_id::HarnessId =
                parse_id(harness_id_str, "harness_id")?;
            let description = get_str(&arguments, "description");
            let agent_id = match get_str(&arguments, "agent_id") {
                Some(value) => Some(parse_id::<everruns_core::typed_id::AgentId>(
                    value, "agent_id",
                )?),
                None => None,
            };
            let agent_identity_id = if let Some(value) = arguments.get("agent_identity_id") {
                if value.is_null() {
                    None
                } else if let Some(value) = value.as_str() {
                    Some(parse_id::<everruns_core::typed_id::AgentIdentityId>(
                        value,
                        "agent_identity_id",
                    )?)
                } else {
                    return Ok(ToolExecutionResult::tool_error(
                        "agent_identity_id must be a string or null",
                    ));
                }
            } else {
                None
            };
            let channel_type = match get_str(&arguments, "channel_type") {
                Some(value) => Some(parse_channel_type(value, "channel_type")?),
                None => None,
            };
            let channel_config = arguments.get("channel_config");

            match store
                .create_app(
                    name,
                    description,
                    harness_id,
                    agent_id,
                    agent_identity_id,
                    channel_type,
                    channel_config,
                )
                .await
            {
                Ok(app) => {
                    let mut response = app_json(&app, base_url, true);
                    response["message"] = Value::String("App created successfully".to_string());
                    ToolExecutionResult::success(response)
                }
                Err(e) => ToolExecutionResult::tool_error(format!("Failed to create app: {e}")),
            }
        }

        "update" => {
            let app_id_str = require_str(&arguments, "app_id")?;
            let app_id: everruns_core::typed_id::AppId = parse_id(app_id_str, "app_id")?;
            let harness_id = match get_str(&arguments, "harness_id") {
                Some(value) => Some(parse_id::<everruns_core::typed_id::HarnessId>(
                    value,
                    "harness_id",
                )?),
                None => None,
            };
            let agent_id = match get_str(&arguments, "agent_id") {
                Some(value) => Some(parse_id::<everruns_core::typed_id::AgentId>(
                    value, "agent_id",
                )?),
                None => None,
            };
            let agent_identity_id = if let Some(value) = arguments.get("agent_identity_id") {
                if value.is_null() {
                    Some(None)
                } else if let Some(value) = value.as_str() {
                    Some(Some(parse_id::<everruns_core::typed_id::AgentIdentityId>(
                        value,
                        "agent_identity_id",
                    )?))
                } else {
                    return Ok(ToolExecutionResult::tool_error(
                        "agent_identity_id must be a string or null",
                    ));
                }
            } else {
                None
            };

            match store
                .update_app(
                    app_id,
                    get_str(&arguments, "name"),
                    get_str(&arguments, "description"),
                    harness_id,
                    agent_id,
                    agent_identity_id,
                )
                .await
            {
                Ok(app) => {
                    let mut response = app_json(&app, base_url, true);
                    response["message"] = Value::String("App updated successfully".to_string());
                    ToolExecutionResult::success(response)
                }
                Err(e) => ToolExecutionResult::tool_error(format!("Failed to update app: {e}")),
            }
        }

        "delete" => {
            let app_id_str = require_str(&arguments, "app_id")?;
            let app_id: everruns_core::typed_id::AppId = parse_id(app_id_str, "app_id")?;
            match store.delete_app(app_id).await {
                Ok(()) => ToolExecutionResult::success(json!({
                    "app_id": app_id_str,
                    "ui_link": format!("{}/apps/{}", base_url, app_id_str),
                    "message": "App archived successfully"
                })),
                Err(e) => ToolExecutionResult::tool_error(format!("Failed to archive app: {e}")),
            }
        }

        "destroy" => {
            let app_id_str = require_str(&arguments, "app_id")?;
            let app_id: everruns_core::typed_id::AppId = parse_id(app_id_str, "app_id")?;
            match store.destroy_app(app_id).await {
                Ok(()) => ToolExecutionResult::success(json!({
                    "app_id": app_id_str,
                    "ui_link": format!("{}/apps", base_url),
                    "message": "App destroyed successfully"
                })),
                Err(e) => ToolExecutionResult::tool_error(format!("Failed to destroy app: {e}")),
            }
        }

        "publish" => {
            let app_id_str = require_str(&arguments, "app_id")?;
            let app_id: everruns_core::typed_id::AppId = parse_id(app_id_str, "app_id")?;
            match store.publish_app(app_id).await {
                Ok(app) => {
                    let mut response = app_json(&app, base_url, true);
                    response["message"] = Value::String("App published successfully".to_string());
                    ToolExecutionResult::success(response)
                }
                Err(e) => ToolExecutionResult::tool_error(format!("Failed to publish app: {e}")),
            }
        }

        "unpublish" => {
            let app_id_str = require_str(&arguments, "app_id")?;
            let app_id: everruns_core::typed_id::AppId = parse_id(app_id_str, "app_id")?;
            match store.unpublish_app(app_id).await {
                Ok(app) => {
                    let mut response = app_json(&app, base_url, true);
                    response["message"] = Value::String("App unpublished successfully".to_string());
                    ToolExecutionResult::success(response)
                }
                Err(e) => ToolExecutionResult::tool_error(format!("Failed to unpublish app: {e}")),
            }
        }

        _ => ToolExecutionResult::tool_error(format!(
            "Unknown operation: {operation}. Valid: create, update, delete, destroy, publish, unpublish"
        )),
    })
}

// =============================================================================
// Tool: manage_app_channels (mutations: add, update, delete)
// =============================================================================

pub struct ManageAppChannelsTool;

#[async_trait]
impl Tool for ManageAppChannelsTool {
    fn name(&self) -> &str {
        "manage_app_channels"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Manage App Channels")
    }

    fn description(&self) -> &str {
        "App channel mutations: add, update, delete."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["add", "update", "delete"],
                    "description": "The channel mutation to perform"
                },
                "app_id": {
                    "type": "string",
                    "description": "App ID"
                },
                "channel_id": {
                    "type": "string",
                    "description": "Channel ID (required for update/delete)"
                },
                "channel_type": {
                    "type": "string",
                    "enum": ["slack", "ag_ui", "schedule", "webhook", "a2a", "fcp"],
                    "description": "Channel type (required for add, optional for update)"
                },
                "channel_config": {
                    "type": "object",
                    "description": "Channel-specific configuration object"
                },
                "enabled": {
                    "type": "boolean",
                    "description": "Whether the channel is enabled"
                }
            },
            "required": ["operation", "app_id"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_narration_noun("app channel")
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "manage_app_channels requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        manage_app_channels_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

async fn manage_app_channels_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let store = get_platform_store(context)?;
    let operation = require_str(&arguments, "operation")?;
    let app_id_str = require_str(&arguments, "app_id")?;
    let app_id: everruns_core::typed_id::AppId = parse_id(app_id_str, "app_id")?;
    let base_url = store.base_url();

    Ok(match operation {
        "add" => {
            let channel_type_str = require_str(&arguments, "channel_type")?;
            let channel_type = parse_channel_type(channel_type_str, "channel_type")?;
            let channel_config = arguments.get("channel_config");
            let enabled = arguments.get("enabled").and_then(|value| value.as_bool());
            match store
                .add_app_channel(app_id, channel_type, channel_config, enabled)
                .await
            {
                Ok(channel) => ToolExecutionResult::success(json!({
                    "app_id": app_id_str,
                    "channel": channel_json(&channel, true),
                    "ui_link": format!("{}/apps/{}", base_url, app_id),
                    "message": "App channel added successfully"
                })),
                Err(e) => {
                    ToolExecutionResult::tool_error(format!("Failed to add app channel: {e}"))
                }
            }
        }

        "update" => {
            let channel_id_str = require_str(&arguments, "channel_id")?;
            let channel_id: everruns_core::typed_id::AppChannelId =
                parse_id(channel_id_str, "channel_id")?;
            let channel_type = match get_str(&arguments, "channel_type") {
                Some(value) => Some(parse_channel_type(value, "channel_type")?),
                None => None,
            };
            let channel_config = arguments.get("channel_config");
            let enabled = arguments.get("enabled").and_then(|value| value.as_bool());
            match store
                .update_app_channel(app_id, channel_id, channel_type, channel_config, enabled)
                .await
            {
                Ok(channel) => ToolExecutionResult::success(json!({
                    "app_id": app_id_str,
                    "channel": channel_json(&channel, true),
                    "ui_link": format!("{}/apps/{}", base_url, app_id),
                    "message": "App channel updated successfully"
                })),
                Err(e) => {
                    ToolExecutionResult::tool_error(format!("Failed to update app channel: {e}"))
                }
            }
        }

        "delete" => {
            let channel_id_str = require_str(&arguments, "channel_id")?;
            let channel_id: everruns_core::typed_id::AppChannelId =
                parse_id(channel_id_str, "channel_id")?;
            match store.delete_app_channel(app_id, channel_id).await {
                Ok(()) => ToolExecutionResult::success(json!({
                    "app_id": app_id_str,
                    "channel_id": channel_id_str,
                    "ui_link": format!("{}/apps/{}", base_url, app_id),
                    "message": "App channel deleted successfully"
                })),
                Err(e) => {
                    ToolExecutionResult::tool_error(format!("Failed to delete app channel: {e}"))
                }
            }
        }

        _ => ToolExecutionResult::tool_error(format!(
            "Unknown operation: {operation}. Valid: add, update, delete"
        )),
    })
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
        read_sessions_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

async fn read_sessions_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let store = get_platform_store(context)?;
    let base_url = store.base_url();

    if let Some(id_str) = get_str(&arguments, "id") {
        let id: everruns_core::typed_id::SessionId = parse_id(id_str, "session id")?;
        return Ok(match store.get_session_by_id(id).await {
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
        });
    }

    let limit = arguments
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);
    // Invalid agent_id is silently ignored here (filter is best-effort).
    let agent_id = get_str(&arguments, "agent_id")
        .and_then(|s| s.parse::<everruns_core::typed_id::AgentId>().ok());
    Ok(match store.list_sessions(limit, agent_id).await {
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
    })
}

// =============================================================================
// Tool: session_context_report
// =============================================================================

pub struct SessionContextReportTool;

#[async_trait]
impl Tool for SessionContextReportTool {
    fn name(&self) -> &str {
        "session_context_report"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Session Context Report")
    }

    fn description(&self) -> &str {
        "Read the latest estimated context token breakdown for a session."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Session ID to inspect"
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
            "session_context_report requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        session_context_report_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

async fn session_context_report_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let store = get_platform_store(context)?;
    let id: everruns_core::typed_id::SessionId = require_id(&arguments, "session_id")?;

    Ok(match store.get_session_context_report(id).await {
        Ok(report) => ToolExecutionResult::success(json!(report)),
        Err(e) => ToolExecutionResult::tool_error(format!("Failed to get context report: {e}")),
    })
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
        ToolHints::default().with_narration_noun("session")
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
        manage_sessions_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

async fn manage_sessions_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let store = get_platform_store(context)?;
    let operation = require_str(&arguments, "operation")?;
    let base_url = store.base_url();

    Ok(match operation {
        "create" => {
            let harness_id = if let Some(harness_id_str) = get_str(&arguments, "harness_id") {
                parse_id::<everruns_core::typed_id::HarnessId>(harness_id_str, "harness_id")?
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
                                return Ok(ToolExecutionResult::tool_error(
                                    "No harness_id provided and no default Generic harness found. Please specify a harness_id.",
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        return Ok(ToolExecutionResult::tool_error(format!(
                            "No harness_id provided and failed to resolve default harness: {e}"
                        )));
                    }
                }
            };
            // Invalid agent_id is silently ignored here (best-effort).
            let agent_id = get_str(&arguments, "agent_id")
                .and_then(|s| s.parse::<everruns_core::typed_id::AgentId>().ok());
            let title = get_str(&arguments, "title");
            let locale = get_str(&arguments, "locale");
            match store
                .create_session(harness_id, agent_id, title, locale, None, None, None)
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
                Err(e) => ToolExecutionResult::tool_error(format!("Failed to create session: {e}")),
            }
        }

        "delete" => {
            let id_str = require_str(&arguments, "session_id")?;
            let id: everruns_core::typed_id::SessionId = parse_id(id_str, "session_id")?;
            match store.delete_session(id).await {
                Ok(()) => ToolExecutionResult::success(json!({
                    "session_id": id_str,
                    "ui_link": format!("{}/sessions/{}/chat", base_url, id_str),
                    "message": "Session archived successfully"
                })),
                Err(e) => ToolExecutionResult::tool_error(format!("Failed to delete session: {e}")),
            }
        }

        _ => ToolExecutionResult::tool_error(format!(
            "Unknown operation: {operation}. Valid: create, delete"
        )),
    })
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
        session_send_message_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

async fn session_send_message_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let store = get_platform_store(context)?;
    let session_id_str = require_str(&arguments, "session_id")?;
    let session_id: everruns_core::typed_id::SessionId = parse_id(session_id_str, "session_id")?;
    let content = require_str(&arguments, "content")?;
    let base_url = store.base_url();

    Ok(match store.send_message(session_id, content).await {
        Ok(()) => ToolExecutionResult::success(json!({
            "session_id": session_id_str,
            "message": "Message sent successfully. Use session_read_response to wait for the agent response.",
            "ui_link": format!("{}/sessions/{}/chat", base_url, session_id),
        })),
        Err(e) => ToolExecutionResult::tool_error(format!("Failed to send message: {e}")),
    })
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
                    "description": "Max messages to return. Default: 10, maximum: 50",
                    "default": SESSION_READ_MESSAGES_DEFAULT_LIMIT,
                    "minimum": 1,
                    "maximum": SESSION_READ_MESSAGES_MAX_LIMIT
                },
                "content_limit": {
                    "type": "integer",
                    "description": "Max characters to return per message. Default: 12000, maximum: 50000",
                    "default": SESSION_READ_MESSAGES_DEFAULT_CONTENT_LIMIT,
                    "minimum": 1,
                    "maximum": SESSION_READ_MESSAGES_MAX_CONTENT_LIMIT
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
        session_read_messages_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

async fn session_read_messages_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let store = get_platform_store(context)?;
    let session_id_str = require_str(&arguments, "session_id")?;
    let session_id: everruns_core::typed_id::SessionId = parse_id(session_id_str, "session_id")?;

    let limit = parse_bounded_usize_arg(
        &arguments,
        "limit",
        SESSION_READ_MESSAGES_DEFAULT_LIMIT,
        SESSION_READ_MESSAGES_MAX_LIMIT,
    )?;
    let content_limit = parse_bounded_usize_arg(
        &arguments,
        "content_limit",
        SESSION_READ_MESSAGES_DEFAULT_CONTENT_LIMIT,
        SESSION_READ_MESSAGES_MAX_CONTENT_LIMIT,
    )?;

    let base_url = store.base_url();

    Ok(match store.get_messages(session_id, Some(limit)).await {
        Ok(messages) => {
            let items: Vec<Value> = messages
                .iter()
                .map(|m| {
                    let (content, truncated, total_chars, returned_chars) =
                        truncate_content_chars(&m.content, content_limit);
                    json!({
                        "role": m.role,
                        "content": content,
                        "content_truncated": truncated,
                        "content_total_chars": total_chars,
                        "content_returned_chars": returned_chars,
                        "created_at": m.created_at.to_rfc3339(),
                    })
                })
                .collect();
            let truncated_message_count = items
                .iter()
                .filter(|item| item["content_truncated"].as_bool().unwrap_or(false))
                .count();
            ToolExecutionResult::success(json!({
                "messages": items,
                "count": items.len(),
                "limit": limit,
                "content_limit": content_limit,
                "truncated_message_count": truncated_message_count,
                "session_id": session_id_str,
                "ui_link": format!("{}/sessions/{}/chat", base_url, session_id),
            }))
        }
        Err(e) => ToolExecutionResult::tool_error(format!("Failed to get messages: {e}")),
    })
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
        session_read_response_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

async fn session_read_response_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let store = get_platform_store(context)?;
    let session_id_str = require_str(&arguments, "session_id")?;
    let session_id: everruns_core::typed_id::SessionId = parse_id(session_id_str, "session_id")?;

    let timeout_secs = arguments.get("timeout_secs").and_then(|v| v.as_u64());
    let base_url = store.base_url();

    Ok(match store.wait_for_idle(session_id, timeout_secs).await {
        Ok(status) => ToolExecutionResult::success(json!({
            "session_id": session_id_str,
            "status": status,
            "ui_link": format!("{}/sessions/{}/chat", base_url, session_id),
        })),
        Err(e) => ToolExecutionResult::tool_error(format!("Failed waiting for response: {e}")),
    })
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
        read_capabilities_impl(arguments, context)
            .await
            .unwrap_or_else(|e| e)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

async fn read_capabilities_impl(
    arguments: Value,
    context: &ToolContext,
) -> Result<ToolExecutionResult, ToolExecutionResult> {
    let store = get_platform_store(context)?;
    let base_url = store.base_url();

    let id_filter = get_str(&arguments, "id");
    let search = get_str(&arguments, "search");

    // If id is given, use it as the search filter to find the specific capability
    let effective_search = id_filter.or(search);

    Ok(match store.list_capabilities(effective_search).await {
        Ok(capabilities) => {
            let items: Vec<Value> = capabilities
                .iter()
                .map(|c| {
                    let mut item = json!({
                        "id": c.id.as_str(),
                        "name": c.name,
                        "description": c.description,
                        "status": c.status.to_string(),
                        "ui_link": format!("{}/capabilities/{}", base_url, c.id.as_str()),
                    });
                    if let Some(cat) = &c.category {
                        item["category"] = json!(cat);
                    }
                    if c.is_mcp {
                        item["type"] = json!("mcp_server");
                    } else if c.is_skill {
                        item["type"] = json!("skill");
                    } else if is_declarative_capability(c.id.as_str()) {
                        item["type"] = json!("declarative");
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
                if let Some(exact) = items.iter().find(|i| i["id"].as_str() == Some(target_id)) {
                    return Ok(ToolExecutionResult::success(exact.clone()));
                }
                return Ok(ToolExecutionResult::tool_error(format!(
                    "Capability not found: {target_id}"
                )));
            }

            let count = items.len();
            ToolExecutionResult::success(json!({
                "capabilities": items,
                "count": count,
                "hint": "Use capability IDs when creating or updating agents and harnesses via manage_agents or manage_harnesses (capabilities parameter)."
            }))
        }
        Err(e) => ToolExecutionResult::tool_error(format!("Failed to list capabilities: {e}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform_store::PlatformStore;
    use crate::platform_store::tests::MockPlatformStore;
    use everruns_core::typed_id::{AgentId, HarnessId, SessionId};
    use std::sync::Arc;

    fn mock_context() -> ToolContext {
        let store: Arc<dyn PlatformStore> = Arc::new(MockPlatformStore::new());
        ToolContext::new(SessionId::new())
            .with_extension(Arc::new(crate::platform_store::PlatformStoreExt(store)))
    }

    // Metadata/tool-list constants covered by builtin_capabilities_satisfy_registry_invariants.

    #[test]
    fn truncate_content_chars_respects_unicode_boundaries() {
        let (content, truncated, total_chars, returned_chars) = truncate_content_chars("ab😀cd", 3);

        assert_eq!(content, "ab😀");
        assert!(truncated);
        assert_eq!(total_chars, 5);
        assert_eq!(returned_chars, 3);
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
                assert_eq!(v["name"], "test-agent");
                assert_eq!(v["display_name"], "Test Agent");
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
                json!({"operation": "create", "name": "new-agent", "system_prompt": "Be helpful"}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => assert_eq!(v["name"], "new-agent"),
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn agent_create_rejects_non_slug_name() {
        let ctx = mock_context();
        let tool = ManageAgentsTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "create", "name": "Bad Agent Name", "system_prompt": "hi"}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::ToolError(_) => {} // expected
            other => panic!("expected tool error for non-slug name, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn agent_create_with_display_name() {
        let ctx = mock_context();
        let tool = ManageAgentsTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "create", "name": "support-bot", "display_name": "Support Bot", "system_prompt": "hi"}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["name"], "support-bot");
                assert_eq!(v["display_name"], "Support Bot");
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
                json!({"operation": "update", "agent_id": AgentId::new().to_string(), "name": "renamed-agent"}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["name"], "renamed-agent");
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
    // ReadAppsTool tests
    // =========================================================================

    #[tokio::test]
    async fn read_apps_list_returns_apps() {
        let ctx = mock_context();
        let tool = ReadAppsTool;
        let result = tool.execute_with_context(json!({}), &ctx).await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["count"], 1);
                assert!(
                    v["apps"].as_array().unwrap()[0]["ui_link"]
                        .as_str()
                        .unwrap()
                        .contains("/apps/")
                );
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_apps_get_by_id_returns_channels() {
        let ctx = mock_context();
        let tool = ReadAppsTool;
        let result = tool
            .execute_with_context(json!({"id": everruns_core::AppId::new().to_string()}), &ctx)
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["name"], "test-app");
                assert_eq!(v["channels"].as_array().unwrap().len(), 1);
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_apps_invalid_id_returns_error() {
        let ctx = mock_context();
        let tool = ReadAppsTool;
        let result = tool.execute_with_context(json!({"id": "bad"}), &ctx).await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Invalid app id")),
            other => panic!("expected tool error, got: {other:?}"),
        }
    }

    // =========================================================================
    // ManageAppsTool tests
    // =========================================================================

    #[tokio::test]
    async fn manage_apps_create_returns_new_app() {
        let ctx = mock_context();
        let tool = ManageAppsTool;
        let result = tool
            .execute_with_context(
                json!({
                    "operation": "create",
                    "name": "repo-checker",
                    "harness_id": HarnessId::new().to_string(),
                    "channel_type": "schedule",
                    "channel_config": {
                        "cron_expression": "0 * * * * * *",
                        "timezone": "UTC",
                        "message": "run checks"
                    }
                }),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["name"], "repo-checker");
                assert_eq!(v["channels"].as_array().unwrap().len(), 1);
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn manage_apps_publish_returns_published_app() {
        let ctx = mock_context();
        let tool = ManageAppsTool;
        let result = tool
            .execute_with_context(
                json!({"operation": "publish", "app_id": everruns_core::AppId::new().to_string()}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => assert_eq!(v["status"], "published"),
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn manage_apps_update_accepts_null_agent_identity() {
        let ctx = mock_context();
        let tool = ManageAppsTool;
        let result = tool
            .execute_with_context(
                json!({
                    "operation": "update",
                    "app_id": everruns_core::AppId::new().to_string(),
                    "agent_identity_id": null
                }),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => assert!(v["agent_identity_id"].is_null()),
            other => panic!("expected success, got: {other:?}"),
        }
    }

    // =========================================================================
    // ManageAppChannelsTool tests
    // =========================================================================

    #[tokio::test]
    async fn manage_app_channels_add_returns_channel() {
        let ctx = mock_context();
        let tool = ManageAppChannelsTool;
        let result = tool
            .execute_with_context(
                json!({
                    "operation": "add",
                    "app_id": everruns_core::AppId::new().to_string(),
                    "channel_type": "webhook",
                    "channel_config": {
                        "token": "secret-1",
                        "message": "process payload"
                    }
                }),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["channel"]["channel_type"], "webhook");
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn manage_app_channels_delete_succeeds() {
        let ctx = mock_context();
        let tool = ManageAppChannelsTool;
        let result = tool
            .execute_with_context(
                json!({
                    "operation": "delete",
                    "app_id": everruns_core::AppId::new().to_string(),
                    "channel_id": everruns_core::AppChannelId::new().to_string()
                }),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert!(v["message"].as_str().unwrap().contains("deleted"));
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn manage_app_channels_invalid_channel_type_returns_error() {
        let ctx = mock_context();
        let tool = ManageAppChannelsTool;
        let result = tool
            .execute_with_context(
                json!({
                    "operation": "add",
                    "app_id": everruns_core::AppId::new().to_string(),
                    "channel_type": "pagerduty"
                }),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Invalid channel_type")),
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

    #[tokio::test]
    async fn session_context_report_returns_report() {
        let ctx = mock_context();
        let tool = SessionContextReportTool;
        let session_id = SessionId::new().to_string();
        let result = tool
            .execute_with_context(json!({"session_id": session_id}), &ctx)
            .await;
        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["estimated_input_tokens"], 42);
                assert_eq!(value["sections"][0]["key"], "conversation");
            }
            other => panic!("expected success, got: {other:?}"),
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
    async fn read_messages_applies_content_limit() {
        let ctx = mock_context();
        let tool = SessionReadMessagesTool;
        let result = tool
            .execute_with_context(
                json!({"session_id": SessionId::new().to_string(), "content_limit": 2}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["content_limit"], 2);
                assert_eq!(v["truncated_message_count"], 2);
                let msgs = v["messages"].as_array().unwrap();
                assert_eq!(msgs[0]["content"], "He");
                assert_eq!(msgs[0]["content_truncated"], true);
                assert_eq!(msgs[0]["content_total_chars"], 5);
                assert_eq!(msgs[0]["content_returned_chars"], 2);
            }
            other => panic!("expected success, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_messages_rejects_zero_limits() {
        let ctx = mock_context();
        let tool = SessionReadMessagesTool;
        let result = tool
            .execute_with_context(
                json!({"session_id": SessionId::new().to_string(), "limit": 0}),
                &ctx,
            )
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("greater than 0")),
            other => panic!("expected tool error, got: {other:?}"),
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
        assert!(ReadAppsTool.requires_context());
        assert!(ManageAppsTool.requires_context());
        assert!(ManageAppChannelsTool.requires_context());
        assert!(ReadSessionsTool.requires_context());
        assert!(SessionContextReportTool.requires_context());
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
            "read_apps",
            "manage_apps",
            "manage_app_channels",
            "read_sessions",
            "session_context_report",
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
                "read_apps" => ReadAppsTool.execute(json!({})).await,
                "manage_apps" => ManageAppsTool.execute(json!({"operation": "create"})).await,
                "manage_app_channels" => {
                    ManageAppChannelsTool
                        .execute(json!({"operation": "add", "app_id": "app_1"}))
                        .await
                }
                "read_sessions" => ReadSessionsTool.execute(json!({})).await,
                "session_context_report" => {
                    SessionContextReportTool
                        .execute(json!({"session_id": "x"}))
                        .await
                }
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
                    assert!(cap["ui_link"].as_str().unwrap().contains("/capabilities/"));
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
            .execute_with_context(json!({"search": "human_intent"}), &ctx)
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                let count = v["count"].as_u64().unwrap();
                assert!(count >= 1, "should find at least human_intent");
                let caps = v["capabilities"].as_array().unwrap();
                assert!(
                    caps.iter()
                        .any(|c| c["id"].as_str().unwrap() == "human_intent"),
                    "should contain human_intent"
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
