// MCP resources as agent tools: `list_mcp_resources` and `read_mcp_resource`.
//
// Spec: knowledge/integrations/mcp-resources.md (D1, D2).
//
// Decision: two shared tools that take a `server` argument, not two per
// server. The tool list stays flat however many servers are attached.
//
// Decision: the names do not start with `mcp_`. A bare `mcp_<x>` is a deferred
// server's placeholder (`mcp_deferred`), and `mcp_<x>__<y>` is a server's own
// tool, so either shape would be routed as something else.
//
// Decision: the tools join the turn whenever it has at least one MCP server,
// deferred servers included. Knowing which servers offer resources would cost a
// `resources/list` round trip per server per turn; a server without resources
// answers "method not found", and the tool says so.

use crate::runtime::mcp_proxy::McpToolInvoker;
use crate::runtime::mcp_server::{
    McpReadResourceResult, McpResourceContents, McpResourceTemplate, McpResourcesPage, is_mcp_tool,
    sanitize_mcp_server_name,
};
use crate::runtime::tool_context::ToolContext;
use crate::runtime::tool_narration::{ToolNarrationContext, ToolNarrationPhase, labeled_phrase};
use crate::runtime::tool_types::{
    BuiltinTool, DeferrablePolicy, ToolCall, ToolDefinition, ToolHints, ToolPolicy, ToolResult,
    ToolResultImage,
};
use crate::runtime::tools::{Tool, ToolExecutionResult};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

/// Lists one MCP server's resources and resource templates.
pub const LIST_MCP_RESOURCES_TOOL: &str = "list_mcp_resources";
/// Reads one resource from an MCP server.
pub const READ_MCP_RESOURCE_TOOL: &str = "read_mcp_resource";

/// Session directory binary resource contents are written to.
const RESOURCE_DIR: &str = "/outputs/mcp-resources";

/// What a resource request against a server came back with.
// Returned and matched immediately, never stored, so the size gap between
// variants costs nothing worth a Box.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum McpResourceOutcome<T> {
    /// The server answered.
    Done(T),
    /// The server needs a connection first; the result says which and where.
    ConnectionRequired(ToolResult),
}

/// One page of a server's resources, with its resource templates on the
/// first page.
#[derive(Debug, Clone, Default)]
pub struct McpResourceListing {
    pub page: McpResourcesPage,
    pub templates: Vec<McpResourceTemplate>,
}

/// True for the two resource tool names.
pub fn is_mcp_resource_tool(name: &str) -> bool {
    name == LIST_MCP_RESOURCES_TOOL || name == READ_MCP_RESOURCE_TOOL
}

/// Add the resource tool definitions when `definitions` holds at least one
/// MCP tool or deferred server and does not have them yet.
pub fn add_mcp_resource_tool_definitions(definitions: &mut Vec<ToolDefinition>) {
    let has_mcp = definitions.iter().any(|def| is_mcp_tool(def.name()));
    let has_tools = definitions
        .iter()
        .any(|def| is_mcp_resource_tool(def.name()));
    if has_mcp && !has_tools {
        definitions.push(list_definition());
        definitions.push(read_definition());
    }
}

/// Registry tools for every resource tool definition in `definitions`.
pub fn build_mcp_resource_tools(
    definitions: &[ToolDefinition],
    invoker: &Arc<dyn McpToolInvoker>,
) -> Vec<Box<dyn Tool>> {
    definitions
        .iter()
        .filter_map(|def| match def.name() {
            LIST_MCP_RESOURCES_TOOL => Some(Box::new(McpResourceTool {
                definition: builtin(list_definition()),
                invoker: invoker.clone(),
            }) as Box<dyn Tool>),
            READ_MCP_RESOURCE_TOOL => Some(Box::new(McpResourceTool {
                definition: builtin(read_definition()),
                invoker: invoker.clone(),
            }) as Box<dyn Tool>),
            _ => None,
        })
        .collect()
}

fn builtin(definition: ToolDefinition) -> BuiltinTool {
    match definition {
        ToolDefinition::Builtin(builtin) => builtin,
        ToolDefinition::ClientSide(_) => unreachable!("resource tools are builtin"),
    }
}

fn server_parameter() -> Value {
    json!({
        "type": "string",
        "description": "The MCP server, as it appears in its tool names: `mcp_<server>__<tool>`."
    })
}

fn list_definition() -> ToolDefinition {
    ToolDefinition::Builtin(BuiltinTool {
        name: LIST_MCP_RESOURCES_TOOL.to_string(),
        display_name: Some("List MCP resources".to_string()),
        description: "List the resources an attached MCP server publishes (files, documents, \
                      records), and its resource templates: URIs with `{placeholders}` you fill \
                      in yourself. Read one with `read_mcp_resource`."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "server": server_parameter(),
                "cursor": {
                    "type": "string",
                    "description": "`next_cursor` from the previous page, to get the next one."
                }
            },
            "required": ["server"],
            "additionalProperties": false
        }),
        policy: ToolPolicy::Auto,
        category: Some("MCP Servers".to_string()),
        deferrable: DeferrablePolicy::Never,
        hints: ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true),
        full_parameters: None,
    })
}

fn read_definition() -> ToolDefinition {
    ToolDefinition::Builtin(BuiltinTool {
        name: READ_MCP_RESOURCE_TOOL.to_string(),
        display_name: Some("Read MCP resource".to_string()),
        description: "Read one resource from an attached MCP server by URI: one listed by \
                      `list_mcp_resources`, a resource template filled in, or a resource link a \
                      tool returned. Text comes back inline, images as images, other binary \
                      files are saved to the session and their path returned."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "server": server_parameter(),
                "uri": { "type": "string", "description": "The resource URI." }
            },
            "required": ["server", "uri"],
            "additionalProperties": false
        }),
        policy: ToolPolicy::Auto,
        category: Some("MCP Servers".to_string()),
        deferrable: DeferrablePolicy::Never,
        hints: ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true),
        full_parameters: None,
    })
}

/// Map one resource's contents into a result part. Text is inlined, an
/// `image/*` blob goes to `images`, other blobs are described by size.
pub fn resource_part(resource: &McpResourceContents, images: &mut Vec<ToolResultImage>) -> Value {
    let mut part = json!({
        "type": "resource",
        "uri": resource.uri,
        "mime_type": resource.mime_type,
    });
    if let Some(text) = &resource.text {
        part["text"] = json!(text);
    } else if let Some(blob) = &resource.blob {
        let mime = resource.mime_type.as_deref().unwrap_or_default();
        if mime.starts_with("image/") {
            images.push(ToolResultImage {
                base64: blob.clone(),
                media_type: mime.to_string(),
            });
            part["image"] = json!(true);
        } else {
            part["size_bytes"] = json!(base64_decoded_len(blob));
        }
    }
    part
}

/// Decoded byte length of a base64 string, without decoding it.
pub fn base64_decoded_len(data: &str) -> usize {
    data.trim_end_matches('=').len() * 3 / 4
}

struct McpResourceTool {
    definition: BuiltinTool,
    invoker: Arc<dyn McpToolInvoker>,
}

impl McpResourceTool {
    fn is_read(&self) -> bool {
        self.definition.name == READ_MCP_RESOURCE_TOOL
    }

    async fn run(
        &self,
        invoker: &dyn McpToolInvoker,
        arguments: &Value,
        context: Option<&ToolContext>,
    ) -> ToolExecutionResult {
        let Some(server) = arguments["server"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
        else {
            return ToolExecutionResult::tool_error("Missing required parameter: server");
        };
        let prefix = sanitize_mcp_server_name(server.trim());
        if self.is_read() {
            let Some(uri) = arguments["uri"].as_str().filter(|u| !u.is_empty()) else {
                return ToolExecutionResult::tool_error("Missing required parameter: uri");
            };
            match invoker.read_resource(&prefix, uri).await {
                Ok(Some(McpResourceOutcome::Done(read))) => {
                    read_result(&prefix, uri, read, context).await
                }
                Ok(Some(McpResourceOutcome::ConnectionRequired(result))) => {
                    connection_required(result)
                }
                Ok(None) => unsupported(),
                Err(error) => ToolExecutionResult::tool_error(error.to_string()),
            }
        } else {
            let cursor = arguments["cursor"].as_str().filter(|c| !c.is_empty());
            match invoker.list_resources(&prefix, cursor).await {
                Ok(Some(McpResourceOutcome::Done(listing))) => list_result(&prefix, listing),
                Ok(Some(McpResourceOutcome::ConnectionRequired(result))) => {
                    connection_required(result)
                }
                Ok(None) => unsupported(),
                Err(error) => ToolExecutionResult::tool_error(error.to_string()),
            }
        }
    }
}

fn unsupported() -> ToolExecutionResult {
    ToolExecutionResult::tool_error("MCP resources are not available on this host.")
}

fn connection_required(result: ToolResult) -> ToolExecutionResult {
    match result.connection_required {
        Some(required) => ToolExecutionResult::ConnectionRequired {
            provider: required.provider,
            subject: required.subject,
            setup_url: required.setup_url,
        },
        None => ToolExecutionResult::tool_error(
            result
                .error
                .unwrap_or_else(|| "The MCP server needs a connection first.".to_string()),
        ),
    }
}

fn list_result(server: &str, listing: McpResourceListing) -> ToolExecutionResult {
    let resources: Vec<Value> = listing
        .page
        .resources
        .iter()
        .map(|r| {
            let mut entry = json!({ "uri": r.uri, "name": r.name });
            let optional = [
                ("title", r.title.clone().map(Value::from)),
                ("description", r.description.clone().map(Value::from)),
                ("mime_type", r.mime_type.clone().map(Value::from)),
                ("size", r.size.map(Value::from)),
                ("annotations", r.annotations.as_ref().map(|a| json!(a))),
            ];
            for (key, value) in optional {
                if let Some(value) = value {
                    entry[key] = value;
                }
            }
            entry
        })
        .collect();
    let templates: Vec<Value> = listing
        .templates
        .iter()
        .map(|t| {
            json!({
                "uri_template": t.uri_template,
                "name": t.name,
                "title": t.title,
                "description": t.description,
                "mime_type": t.mime_type,
            })
        })
        .collect();
    let mut result = json!({ "server": server, "resources": resources });
    if !templates.is_empty() {
        result["resource_templates"] = json!(templates);
    }
    if let Some(cursor) = listing.page.next_cursor {
        result["next_cursor"] = json!(cursor);
    }
    ToolExecutionResult::Success(result)
}

async fn read_result(
    server: &str,
    uri: &str,
    read: McpReadResourceResult,
    context: Option<&ToolContext>,
) -> ToolExecutionResult {
    let mut images = Vec::new();
    let mut parts = Vec::new();
    for (index, contents) in read.contents.iter().enumerate() {
        let mut part = resource_part(contents, &mut images);
        if part.get("size_bytes").is_some()
            && let (Some(blob), Some(context)) = (&contents.blob, context)
            && let Some(path) = save_blob(context, index, contents, blob).await
        {
            part["path"] = json!(path);
        }
        parts.push(part);
    }
    let result = json!({ "server": server, "uri": uri, "contents": parts });
    if images.is_empty() {
        ToolExecutionResult::Success(result)
    } else {
        ToolExecutionResult::SuccessWithImages { result, images }
    }
}

/// Write a binary resource to the session's files, returning its path.
async fn save_blob(
    context: &ToolContext,
    index: usize,
    contents: &McpResourceContents,
    blob: &str,
) -> Option<String> {
    let file_store = context.file_store.as_ref()?;
    let call: String = context
        .tool_call_id
        .as_deref()
        .unwrap_or("read")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let name: String = contents
        .uri
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    let name = if name.trim_matches('.').is_empty() {
        "resource.bin".to_string()
    } else {
        name
    };
    let path = format!("{RESOURCE_DIR}/{call}-{index}-{name}");
    file_store
        .write_file(context.session_id, &path, blob, "base64")
        .await
        .ok()
        .map(|_| path)
}

#[async_trait]
impl Tool for McpResourceTool {
    fn name(&self) -> &str {
        &self.definition.name
    }

    fn display_name(&self) -> Option<&str> {
        self.definition.display_name.as_deref()
    }

    fn description(&self) -> &str {
        &self.definition.description
    }

    fn parameters_schema(&self) -> Value {
        self.definition.parameters.clone()
    }

    fn hints(&self) -> ToolHints {
        self.definition.hints.clone()
    }

    fn deferrable_policy(&self) -> DeferrablePolicy {
        DeferrablePolicy::Never
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn to_definition(&self) -> ToolDefinition {
        ToolDefinition::Builtin(self.definition.clone())
    }

    fn narrate(
        &self,
        tool_call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        let value = if self.is_read() {
            tool_call.arguments["uri"].as_str().map(str::to_string)
        } else {
            tool_call.arguments["server"].as_str().map(str::to_string)
        };
        let uk = locale.is_some_and(|l| l.starts_with("uk"));
        let phrase = match (self.is_read(), uk) {
            (true, true) => labeled_phrase(
                "Читаю ресурс MCP",
                "Прочитав ресурс MCP",
                "Не вдалося прочитати ресурс MCP",
                value,
                phase,
            ),
            (true, false) => labeled_phrase(
                "Reading MCP resource",
                "Read MCP resource",
                "Could not read MCP resource",
                value,
                phase,
            ),
            (false, true) => labeled_phrase(
                "Переглядаю ресурси MCP-сервера",
                "Переглянув ресурси MCP-сервера",
                "Не вдалося переглянути ресурси MCP-сервера",
                value,
                phase,
            ),
            (false, false) => labeled_phrase(
                "Listing MCP server resources",
                "Listed MCP server resources",
                "Could not list MCP server resources",
                value,
                phase,
            ),
        };
        Some(phrase)
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        self.run(self.invoker.as_ref(), &arguments, None).await
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        // Same credential binding as an MCP tool call on that server.
        let scoped = context
            .event_context
            .as_ref()
            .and_then(|c| c.input_message_id)
            .and_then(|id| self.invoker.for_execution(id.uuid()));
        let invoker = scoped.as_deref().unwrap_or(self.invoker.as_ref());
        self.run(invoker, &arguments, Some(context)).await
    }
}

#[cfg(test)]
#[path = "mcp_resources_tests.rs"]
mod tests;
