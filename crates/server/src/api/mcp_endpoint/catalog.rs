// API operations catalog — each entry becomes a ScriptedTool builtin
//
// Every Operation has a `name` used as the bash command name in the
// ScriptedTool interpreter, plus enough metadata to generate the ToolDef
// and the direct service callback.

use super::handlers;
use bashkit::{ScriptedTool, ToolArgs, ToolDef};
use serde_json::json;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
/// Handler function type for catalog operations.
pub type Handler = for<'a> fn(
    &'a serde_json::Value,
    &'a CatalogContext,
) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>>;

/// A single API operation in the catalog.
pub struct Operation {
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub params: &'static [Param],
    pub handler: Handler,
}

/// A parameter for an API operation.
pub struct Param {
    pub name: &'static str,
    pub typ: &'static str,
    pub description: &'static str,
}

/// Shared context for direct service calls from catalog operations.
#[derive(Clone)]
pub struct CatalogContext {
    pub state: super::AppState,
    pub caller: everruns_core::permissions::Caller,
    pub org_id: i64,
    pub user_id: Option<uuid::Uuid>,
}

fn tool_builder() -> bashkit::ScriptedToolBuilder {
    ScriptedTool::builder("everruns")
        .short_description("Everruns API operations as bash builtins")
        .limits(
            bashkit::ExecutionLimits::new()
                .max_commands(500)
                .max_loop_iterations(5000)
                .max_function_depth(50)
                .max_input_bytes(500_000)
                .max_ast_depth(50)
                .parser_timeout(std::time::Duration::from_secs(3)),
        )
}

/// Build a ScriptedTool with direct service calls for all catalog operations.
pub fn build_scripted_tool(ctx: CatalogContext) -> ScriptedTool {
    let mut builder = tool_builder();

    for op in CATALOG {
        let def = op_to_def(op);
        let callback = make_direct_callback(op, ctx.clone());
        builder = builder.tool(def, callback);
    }

    builder.build()
}

/// Create a callback that calls a handler function directly via service layer.
fn make_direct_callback(
    op: &'static Operation,
    ctx: CatalogContext,
) -> impl Fn(&ToolArgs) -> Result<String, String> + Send + Sync + 'static {
    move |args: &ToolArgs| {
        let params = &args.params;

        if is_flag_set(params, "help") {
            return Ok(format_help(op));
        }

        let wants_summary = is_flag_set(params, "summary");

        tokio::task::block_in_place(|| {
            let handle = tokio::runtime::Handle::current();
            handle.block_on(async {
                let result = (op.handler)(params, &ctx).await?;
                if wants_summary {
                    Ok(apply_summary_filter(&result))
                } else {
                    Ok(result)
                }
            })
        })
    }
}

/// Convert an Operation to a bashkit ToolDef.
fn op_to_def(op: &Operation) -> ToolDef {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();

    for p in op.params {
        let prop_type = match p.typ {
            "array" => "array",
            "object" => "object",
            "integer" => "integer",
            "boolean" => "boolean",
            _ => "string", // path, query, string all become string
        };
        properties.insert(
            p.name.to_string(),
            json!({ "type": prop_type, "description": p.description }),
        );
        // Path params are always required
        if p.typ == "path" {
            required.push(p.name.to_string());
        }
    }

    let schema = json!({
        "type": "object",
        "properties": properties,
        "required": required,
    });

    ToolDef::new(op.name, op.description)
        .with_schema(schema)
        .with_category(op.category)
}

/// Map catalog param type to display type (same logic as op_to_def).
fn display_type(typ: &str) -> &str {
    match typ {
        "array" => "array",
        "object" => "object",
        "integer" => "integer",
        "boolean" => "boolean",
        _ => "string", // path, query, string all display as string
    }
}

/// Client-side flags that are never forwarded to the API.
const CLIENT_FLAGS: &[&str] = &["summary", "help"];

/// Generate local --help text for an operation.
fn format_help(op: &Operation) -> String {
    let mut out = String::new();
    out.push_str(&format!("{} — {}\n\n", op.name, op.description));
    out.push_str(&format!("Usage: {} [OPTIONS]\n\n", op.name));

    let api_params: Vec<&Param> = op
        .params
        .iter()
        .filter(|p| !CLIENT_FLAGS.contains(&p.name))
        .collect();
    if !api_params.is_empty() {
        out.push_str("Options:\n");
        for p in api_params {
            let required = if p.typ == "path" { " (required)" } else { "" };
            out.push_str(&format!(
                "  --{:<24} {}  [{}{}]\n",
                p.name,
                p.description,
                display_type(p.typ),
                required
            ));
        }
        out.push('\n');
    }

    out.push_str("Flags:\n");
    out.push_str("  --help                     Show this help message\n");
    out.push_str("  --summary                  Return only id, name, description, status fields\n");
    out
}

/// Check if a flag-like param is truthy (bool true or string "true").
fn is_flag_set(params: &serde_json::Value, key: &str) -> bool {
    params
        .get(key)
        .is_some_and(|v| v.as_bool().unwrap_or(false) || v.as_str().is_some_and(|s| s == "true"))
}

/// Summary fields kept in compact output mode.
const SUMMARY_FIELDS: &[&str] = &[
    "id",
    "name",
    "description",
    "status",
    "self_url",
    "view_url",
];

/// Filter a JSON response to summary fields only.
///
/// Expects a response with a `data` array. Each item in the array is stripped to
/// only contain the fields in `SUMMARY_FIELDS`. If the response doesn't have a
/// `data` array, returns the original body unchanged.
fn apply_summary_filter(body: &str) -> String {
    let Ok(mut parsed) = serde_json::from_str::<serde_json::Value>(body) else {
        return body.to_string();
    };

    if let Some(data) = parsed.get_mut("data").and_then(|d| d.as_array_mut()) {
        for item in data.iter_mut() {
            if let Some(obj) = item.as_object_mut() {
                obj.retain(|key, _| SUMMARY_FIELDS.contains(&key.as_str()));
            }
        }
    }

    serde_json::to_string(&parsed).unwrap_or_else(|_| body.to_string())
}

// ============================================================================
// Catalog entries
// ============================================================================

pub static CATALOG: &[Operation] = &[
    // ── Agents ──────────────────────────────────────────────────────────
    Operation {
        name: "create_agent",
        category: "agents",
        description: "Create a new agent with a name, system prompt, and optional capabilities.",
        params: &[
            Param {
                name: "name",
                typ: "string",
                description: "Addressable agent name (required). Lowercase letters, numbers, and hyphens only (e.g. 'customer-support').",
            },
            Param {
                name: "system_prompt",
                typ: "string",
                description: "System prompt for the agent",
            },
            Param {
                name: "capabilities",
                typ: "array",
                description: "Capability configs [{\"ref\": \"...\"}]",
            },
            Param {
                name: "default_model_id",
                typ: "string",
                description: "Default model ID",
            },
        ],
        handler: |p, c| Box::pin(handlers::create_agent(p, c)),
    },
    Operation {
        name: "list_agents",
        category: "agents",
        description: "List all active agents. Use search for name search, include_archived=true to include archived. Supports pagination (limit/offset) and --summary for compact output.",
        params: &[
            Param {
                name: "search",
                typ: "query",
                description: "Search by name (optional)",
            },
            Param {
                name: "include_archived",
                typ: "query",
                description: "Include archived agents (default: false)",
            },
            Param {
                name: "offset",
                typ: "query",
                description: "Pagination offset (default: 0)",
            },
            Param {
                name: "limit",
                typ: "query",
                description: "Page size (default: 20, max: 100)",
            },
            Param {
                name: "summary",
                typ: "query",
                description: "Compact output: id, name, description, status, self_url, view_url (default: false)",
            },
        ],
        handler: |p, c| Box::pin(handlers::list_agents(p, c)),
    },
    Operation {
        name: "get_agent",
        category: "agents",
        description: "Get a single agent by ID.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Agent ID (format: agent_{32-hex})",
        }],
        handler: |p, c| Box::pin(handlers::get_agent(p, c)),
    },
    Operation {
        name: "update_agent",
        category: "agents",
        description: "Update an agent. Only provided fields are changed.",
        params: &[
            Param {
                name: "id",
                typ: "path",
                description: "Agent ID",
            },
            Param {
                name: "name",
                typ: "string",
                description: "New addressable name. Lowercase letters, numbers, and hyphens only.",
            },
            Param {
                name: "system_prompt",
                typ: "string",
                description: "New system prompt",
            },
        ],
        handler: |p, c| Box::pin(handlers::update_agent(p, c)),
    },
    Operation {
        name: "delete_agent",
        category: "agents",
        description: "Archive an agent (soft delete). Can be restored.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Agent ID",
        }],
        handler: |p, c| Box::pin(handlers::delete_agent(p, c)),
    },
    Operation {
        name: "upsert_agent",
        category: "agents",
        description: "Upsert agent — create (201) or update (200) by ID.",
        params: &[
            Param {
                name: "id",
                typ: "path",
                description: "Agent ID",
            },
            Param {
                name: "name",
                typ: "string",
                description: "Agent name",
            },
            Param {
                name: "system_prompt",
                typ: "string",
                description: "System prompt",
            },
        ],
        handler: |p, c| Box::pin(handlers::upsert_agent(p, c)),
    },
    Operation {
        name: "copy_agent",
        category: "agents",
        description: "Copy an agent with a new ID and '{name} (copy)' name.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Source agent ID",
        }],
        handler: |p, c| Box::pin(handlers::copy_agent(p, c)),
    },
    Operation {
        name: "export_agent",
        category: "agents",
        description: "Export agent definition as Markdown.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Agent ID",
        }],
        handler: |p, c| Box::pin(handlers::export_agent(p, c)),
    },
    Operation {
        name: "import_agent",
        category: "agents",
        description: "Import agent from Markdown file content.",
        params: &[Param {
            name: "content",
            typ: "string",
            description: "Markdown content to import",
        }],
        handler: |p, c| Box::pin(handlers::import_agent(p, c)),
    },
    Operation {
        name: "preview_agent",
        category: "agents",
        description: "Preview final agent shape (system prompt + tools) without persisting.",
        params: &[
            Param {
                name: "system_prompt",
                typ: "string",
                description: "System prompt",
            },
            Param {
                name: "capabilities",
                typ: "array",
                description: "Capability configs",
            },
        ],
        handler: |p, c| Box::pin(handlers::preview_agent(p, c)),
    },
    // ── Sessions ────────────────────────────────────────────────────────
    Operation {
        name: "create_session",
        category: "sessions",
        description: "Create a new session. Optionally assign an agent and harness.",
        params: &[
            Param {
                name: "agent_id",
                typ: "string",
                description: "Agent ID (optional)",
            },
            Param {
                name: "harness_id",
                typ: "string",
                description: "Harness ID (optional, defaults to org base)",
            },
            Param {
                name: "title",
                typ: "string",
                description: "Session title",
            },
            Param {
                name: "model_id",
                typ: "string",
                description: "Model override",
            },
        ],
        handler: |p, c| Box::pin(handlers::create_session(p, c)),
    },
    Operation {
        name: "list_sessions",
        category: "sessions",
        description: "List sessions. Filter by agent_id, search by title. Supports pagination (limit/offset) and --summary for compact output.",
        params: &[
            Param {
                name: "agent_id",
                typ: "query",
                description: "Filter by agent (optional)",
            },
            Param {
                name: "search",
                typ: "query",
                description: "Search by title (optional)",
            },
            Param {
                name: "offset",
                typ: "query",
                description: "Pagination offset (default: 0)",
            },
            Param {
                name: "limit",
                typ: "query",
                description: "Page size (default: 20, max: 100)",
            },
            Param {
                name: "summary",
                typ: "query",
                description: "Compact output: id, name, description, status, self_url, view_url (default: false)",
            },
        ],
        handler: |p, c| Box::pin(handlers::list_sessions(p, c)),
    },
    Operation {
        name: "get_session",
        category: "sessions",
        description: "Get session details including status, agent, harness, and model.",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
        handler: |p, c| Box::pin(handlers::get_session(p, c)),
    },
    Operation {
        name: "update_session",
        category: "sessions",
        description: "Update session title, tags, or locale.",
        params: &[
            Param {
                name: "session_id",
                typ: "path",
                description: "Session ID",
            },
            Param {
                name: "title",
                typ: "string",
                description: "New title",
            },
            Param {
                name: "tags",
                typ: "array",
                description: "New tags",
            },
        ],
        handler: |p, c| Box::pin(handlers::update_session(p, c)),
    },
    Operation {
        name: "delete_session",
        category: "sessions",
        description: "Delete a session.",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
        handler: |p, c| Box::pin(handlers::delete_session(p, c)),
    },
    Operation {
        name: "cancel_session",
        category: "sessions",
        description: "Cancel the currently executing turn in a session.",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
        handler: |p, c| Box::pin(handlers::cancel_session(p, c)),
    },
    // ── Budgets ─────────────────────────────────────────────────────────
    Operation {
        name: "create_budget",
        category: "budgets",
        description: "Create a budget for a subject (session, agent, user, org). Sets a spending cap in the given currency.",
        params: &[
            Param {
                name: "subject_type",
                typ: "string",
                description: "Subject type: session, agent, user, or org",
            },
            Param {
                name: "subject_id",
                typ: "string",
                description: "Subject ID (e.g. session ID, agent ID)",
            },
            Param {
                name: "currency",
                typ: "string",
                description: "Budget currency: usd, tokens, credits, or custom",
            },
            Param {
                name: "limit",
                typ: "number",
                description: "Hard spending limit",
            },
            Param {
                name: "soft_limit",
                typ: "number",
                description: "Optional soft limit (pauses before hard stop)",
            },
            Param {
                name: "period",
                typ: "string",
                description: "Optional period as JSON (e.g. {\"type\":\"calendar\",\"unit\":\"month\"})",
            },
            Param {
                name: "metadata",
                typ: "string",
                description: "Optional metadata as JSON",
            },
        ],
        handler: |p, c| Box::pin(handlers::create_budget(p, c)),
    },
    Operation {
        name: "list_budgets",
        category: "budgets",
        description: "List budgets. Filter by subject_type and subject_id.",
        params: &[
            Param {
                name: "subject_type",
                typ: "query",
                description: "Filter by subject type (optional)",
            },
            Param {
                name: "subject_id",
                typ: "query",
                description: "Filter by subject ID (optional)",
            },
        ],
        handler: |p, c| Box::pin(handlers::list_budgets(p, c)),
    },
    Operation {
        name: "get_budget",
        category: "budgets",
        description: "Get a budget with current balance.",
        params: &[Param {
            name: "budget_id",
            typ: "path",
            description: "Budget ID",
        }],
        handler: |p, c| Box::pin(handlers::get_budget(p, c)),
    },
    Operation {
        name: "update_budget",
        category: "budgets",
        description: "Update a budget's limit, soft_limit, or status.",
        params: &[
            Param {
                name: "budget_id",
                typ: "path",
                description: "Budget ID",
            },
            Param {
                name: "limit",
                typ: "number",
                description: "New hard limit (optional)",
            },
            Param {
                name: "soft_limit",
                typ: "number",
                description: "New soft limit (optional)",
            },
            Param {
                name: "status",
                typ: "string",
                description: "New status: active, disabled (optional)",
            },
            Param {
                name: "metadata",
                typ: "string",
                description: "Optional metadata as JSON",
            },
        ],
        handler: |p, c| Box::pin(handlers::update_budget(p, c)),
    },
    Operation {
        name: "delete_budget",
        category: "budgets",
        description: "Soft-delete a budget (sets status to disabled).",
        params: &[Param {
            name: "budget_id",
            typ: "path",
            description: "Budget ID",
        }],
        handler: |p, c| Box::pin(handlers::delete_budget(p, c)),
    },
    Operation {
        name: "top_up_budget",
        category: "budgets",
        description: "Add credits to a budget. Reactivates exhausted/paused budgets if balance becomes positive.",
        params: &[
            Param {
                name: "budget_id",
                typ: "path",
                description: "Budget ID",
            },
            Param {
                name: "amount",
                typ: "number",
                description: "Amount to add",
            },
            Param {
                name: "description",
                typ: "string",
                description: "Optional description for the top-up",
            },
        ],
        handler: |p, c| Box::pin(handlers::top_up_budget(p, c)),
    },
    Operation {
        name: "check_budget",
        category: "budgets",
        description: "Check budget status and remaining balance.",
        params: &[Param {
            name: "budget_id",
            typ: "path",
            description: "Budget ID",
        }],
        handler: |p, c| Box::pin(handlers::check_budget(p, c)),
    },
    Operation {
        name: "list_session_budgets",
        category: "budgets",
        description: "List all budgets for a session.",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
        handler: |p, c| Box::pin(handlers::list_session_budgets(p, c)),
    },
    Operation {
        name: "check_session_budgets",
        category: "budgets",
        description: "Check all budgets for a session (currently includes session and agent scopes).",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
        handler: |p, c| Box::pin(handlers::check_session_budgets(p, c)),
    },
    // ── Messages ────────────────────────────────────────────────────────
    Operation {
        name: "create_message",
        category: "messages",
        description: "Create a user message in a session. Triggers the agent workflow.",
        params: &[
            Param {
                name: "session_id",
                typ: "path",
                description: "Session ID",
            },
            Param {
                name: "message",
                typ: "object",
                description: "{\"role\": \"user\", \"content\": [{\"type\": \"text\", \"text\": \"...\"}]}",
            },
        ],
        handler: |p, c| Box::pin(handlers::create_message(p, c)),
    },
    Operation {
        name: "list_messages",
        category: "messages",
        description: "List all messages in a session (user and agent messages).",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
        handler: |p, c| Box::pin(handlers::list_messages(p, c)),
    },
    // ── Events ──────────────────────────────────────────────────────────
    Operation {
        name: "list_events",
        category: "events",
        description: "List events for a session (JSON). Supports filtering by type and pagination.",
        params: &[
            Param {
                name: "session_id",
                typ: "path",
                description: "Session ID",
            },
            Param {
                name: "since_id",
                typ: "query",
                description: "Return events after this event ID",
            },
            Param {
                name: "types",
                typ: "query",
                description: "Filter by event types (repeatable)",
            },
            Param {
                name: "exclude",
                typ: "query",
                description: "Exclude event types (repeatable)",
            },
            Param {
                name: "limit",
                typ: "query",
                description: "Max events (1-1000)",
            },
        ],
        handler: |p, c| Box::pin(handlers::list_events(p, c)),
    },
    Operation {
        name: "stream_sse",
        category: "events",
        description: "Stream events via SSE (Server-Sent Events). Real-time event streaming.",
        params: &[
            Param {
                name: "session_id",
                typ: "path",
                description: "Session ID",
            },
            Param {
                name: "since_id",
                typ: "query",
                description: "Resume from event ID",
            },
            Param {
                name: "types",
                typ: "query",
                description: "Filter by event types",
            },
        ],
        handler: |p, c| Box::pin(handlers::stream_sse(p, c)),
    },
    // ── Harnesses ───────────────────────────────────────────────────────
    Operation {
        name: "list_harnesses",
        category: "harnesses",
        description: "List harnesses (base environments for sessions). Supports --summary for compact output.",
        params: &[
            Param {
                name: "include_archived",
                typ: "query",
                description: "Include archived (default: false)",
            },
            Param {
                name: "summary",
                typ: "query",
                description: "Compact output: id, name, description, status, self_url, view_url (default: false)",
            },
        ],
        handler: |p, c| Box::pin(handlers::list_harnesses(p, c)),
    },
    Operation {
        name: "get_harness",
        category: "harnesses",
        description: "Get a harness by ID.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Harness ID",
        }],
        handler: |p, c| Box::pin(handlers::get_harness(p, c)),
    },
    // ── LLM Models ──────────────────────────────────────────────────────
    Operation {
        name: "list_models",
        category: "models",
        description: "List all LLM models across all providers. Supports --summary for compact output.",
        params: &[Param {
            name: "summary",
            typ: "query",
            description: "Compact output: id, name, description, status, self_url, view_url (default: false)",
        }],
        handler: |p, c| Box::pin(handlers::list_models(p, c)),
    },
    Operation {
        name: "get_model",
        category: "models",
        description: "Get a single LLM model by ID.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Model ID",
        }],
        handler: |p, c| Box::pin(handlers::get_model(p, c)),
    },
    // ── LLM Providers ───────────────────────────────────────────────────
    Operation {
        name: "list_providers",
        category: "providers",
        description: "List configured LLM providers (OpenAI, Anthropic, etc.). Supports --summary for compact output.",
        params: &[Param {
            name: "summary",
            typ: "query",
            description: "Compact output: id, name, description, status, self_url, view_url (default: false)",
        }],
        handler: |p, c| Box::pin(handlers::list_providers(p, c)),
    },
    Operation {
        name: "create_provider",
        category: "providers",
        description: "Create a new LLM provider configuration.",
        params: &[
            Param {
                name: "name",
                typ: "string",
                description: "Provider name",
            },
            Param {
                name: "provider_type",
                typ: "string",
                description: "openai, anthropic, google, azure_openai, custom",
            },
            Param {
                name: "api_key",
                typ: "string",
                description: "API key for the provider",
            },
        ],
        handler: |p, c| Box::pin(handlers::create_provider(p, c)),
    },
    // ── MCP Servers ─────────────────────────────────────────────────────
    Operation {
        name: "list_mcp_servers",
        category: "mcp_servers",
        description: "List registered MCP servers. Supports --summary for compact output.",
        params: &[
            Param {
                name: "search",
                typ: "query",
                description: "Search by name",
            },
            Param {
                name: "summary",
                typ: "query",
                description: "Compact output: id, name, description, status, self_url, view_url (default: false)",
            },
        ],
        handler: |p, c| Box::pin(handlers::list_mcp_servers(p, c)),
    },
    Operation {
        name: "create_mcp_server",
        category: "mcp_servers",
        description: "Register a new MCP server.",
        params: &[
            Param {
                name: "name",
                typ: "string",
                description: "Server name (unique per org)",
            },
            Param {
                name: "url",
                typ: "string",
                description: "Server URL (HTTP)",
            },
            Param {
                name: "description",
                typ: "string",
                description: "Description (optional)",
            },
        ],
        handler: |p, c| Box::pin(handlers::create_mcp_server(p, c)),
    },
    Operation {
        name: "get_mcp_server",
        category: "mcp_servers",
        description: "Get an MCP server by ID.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "MCP server ID",
        }],
        handler: |p, c| Box::pin(handlers::get_mcp_server(p, c)),
    },
    // ── Capabilities ────────────────────────────────────────────────────
    Operation {
        name: "list_capabilities",
        category: "capabilities",
        description: "List available capabilities (virtual bash, web fetch, MCP, etc.). Supports search, pagination (limit/offset), and --summary for compact output.",
        params: &[
            Param {
                name: "search",
                typ: "query",
                description: "Search by name or description",
            },
            Param {
                name: "offset",
                typ: "query",
                description: "Pagination offset (default: 0)",
            },
            Param {
                name: "limit",
                typ: "query",
                description: "Page size (default: 100, max: 200)",
            },
            Param {
                name: "summary",
                typ: "query",
                description: "Compact output: id, name, description, status, self_url, view_url (default: false)",
            },
        ],
        handler: |p, c| Box::pin(handlers::list_capabilities(p, c)),
    },
    Operation {
        name: "get_capability",
        category: "capabilities",
        description: "Get capability details by ID.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Capability ID",
        }],
        handler: |p, c| Box::pin(handlers::get_capability(p, c)),
    },
    // ── Skills ──────────────────────────────────────────────────────────
    Operation {
        name: "list_skills",
        category: "skills",
        description: "List registered skills. Supports --summary for compact output.",
        params: &[
            Param {
                name: "include_archived",
                typ: "query",
                description: "Include archived",
            },
            Param {
                name: "summary",
                typ: "query",
                description: "Compact output: id, name, description, status, self_url, view_url (default: false)",
            },
        ],
        handler: |p, c| Box::pin(handlers::list_skills(p, c)),
    },
    Operation {
        name: "create_skill",
        category: "skills",
        description: "Create a new skill.",
        params: &[
            Param {
                name: "name",
                typ: "string",
                description: "Skill name",
            },
            Param {
                name: "description",
                typ: "string",
                description: "Description",
            },
            Param {
                name: "source_type",
                typ: "string",
                description: "inline, url, or github",
            },
        ],
        handler: |p, c| Box::pin(handlers::create_skill(p, c)),
    },
    // ── Images ──────────────────────────────────────────────────────────
    Operation {
        name: "list_images",
        category: "images",
        description: "List uploaded images. Supports pagination (limit/offset) and --summary for compact output.",
        params: &[
            Param {
                name: "offset",
                typ: "query",
                description: "Pagination offset (default: 0)",
            },
            Param {
                name: "limit",
                typ: "query",
                description: "Page size (default: 50, max: 100)",
            },
            Param {
                name: "summary",
                typ: "query",
                description: "Compact output: id, name, description, status, self_url, view_url (default: false)",
            },
        ],
        handler: |p, c| Box::pin(handlers::list_images(p, c)),
    },
    Operation {
        name: "get_image",
        category: "images",
        description: "Get image data by ID.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Image ID",
        }],
        handler: |p, c| Box::pin(handlers::get_image(p, c)),
    },
    // ── Schedules ───────────────────────────────────────────────────────
    Operation {
        name: "list_schedules",
        category: "schedules",
        description: "List durable scheduled tasks. Supports --summary for compact output.",
        params: &[Param {
            name: "summary",
            typ: "query",
            description: "Compact output: id, name, description, status, self_url, view_url (default: false)",
        }],
        handler: |p, c| Box::pin(handlers::list_schedules(p, c)),
    },
    Operation {
        name: "create_schedule",
        category: "schedules",
        description: "Create a new durable scheduled task with a cron expression.",
        params: &[
            Param {
                name: "name",
                typ: "string",
                description: "Schedule name",
            },
            Param {
                name: "cron",
                typ: "string",
                description: "Cron expression",
            },
            Param {
                name: "target",
                typ: "object",
                description: "Schedule target configuration",
            },
        ],
        handler: |p, c| Box::pin(handlers::create_schedule(p, c)),
    },
    // ── Organizations ───────────────────────────────────────────────────
    Operation {
        name: "list_orgs",
        category: "organizations",
        description: "List organizations for the current user. Supports --summary for compact output.",
        params: &[Param {
            name: "summary",
            typ: "query",
            description: "Compact output: id, name, description, status, self_url, view_url (default: false)",
        }],
        handler: |p, c| Box::pin(handlers::list_orgs(p, c)),
    },
    Operation {
        name: "get_org",
        category: "organizations",
        description: "Get organization details.",
        params: &[Param {
            name: "org",
            typ: "path",
            description: "Organization ID or slug",
        }],
        handler: |p, c| Box::pin(handlers::get_org(p, c)),
    },
    // ── Users ───────────────────────────────────────────────────────────
    Operation {
        name: "list_users",
        category: "users",
        description: "List users in the current organization. Supports search filtering and --summary for compact output.",
        params: &[
            Param {
                name: "search",
                typ: "query",
                description: "Filter users by search term",
            },
            Param {
                name: "summary",
                typ: "query",
                description: "Compact output: id, name, description, status, self_url, view_url (default: false)",
            },
        ],
        handler: |p, c| Box::pin(handlers::list_users(p, c)),
    },
    // ── Session Files ───────────────────────────────────────────────────
    Operation {
        name: "list_session_files",
        category: "files",
        description: "Get the root directory listing of session files.",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
        handler: |p, c| Box::pin(handlers::list_session_files(p, c)),
    },
    Operation {
        name: "get_session_file",
        category: "files",
        description: "Get a file or directory at a path in the session filesystem.",
        params: &[
            Param {
                name: "session_id",
                typ: "path",
                description: "Session ID",
            },
            Param {
                name: "path",
                typ: "path",
                description: "File path",
            },
        ],
        handler: |p, c| Box::pin(handlers::get_session_file(p, c)),
    },
    // ── Session Databases ───────────────────────────────────────────────
    Operation {
        name: "list_session_databases",
        category: "databases",
        description: "List session-scoped SQL databases.",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
        handler: |p, c| Box::pin(handlers::list_session_databases(p, c)),
    },
    // ── Session Storage ─────────────────────────────────────────────────
    Operation {
        name: "list_session_storage",
        category: "storage",
        description: "List key-value pairs in session storage.",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
        handler: |p, c| Box::pin(handlers::list_session_storage(p, c)),
    },
    // ── Tool Results ────────────────────────────────────────────────────
    Operation {
        name: "submit_tool_results",
        category: "tool_results",
        description: "Submit client-side tool results back to a waiting session.",
        params: &[
            Param {
                name: "session_id",
                typ: "path",
                description: "Session ID",
            },
            Param {
                name: "results",
                typ: "array",
                description: "Array of {tool_call_id, output} objects",
            },
        ],
        handler: |p, c| Box::pin(handlers::submit_tool_results(p, c)),
    },
    // ── Health ──────────────────────────────────────────────────────────
    Operation {
        name: "health_check",
        category: "system",
        description: "Health check endpoint. Returns server version and runner mode.",
        params: &[],
        handler: |p, c| Box::pin(handlers::health_check(p, c)),
    },
];

// ============================================================================
// Discover — fuzzy search and catalog listing
// ============================================================================

/// Return all tools grouped by category.
pub fn discover_all() -> String {
    let mut by_category: BTreeMap<&str, Vec<&Operation>> = BTreeMap::new();
    for op in CATALOG {
        by_category.entry(op.category).or_default().push(op);
    }

    let mut out = String::new();
    for (cat, ops) in &by_category {
        out.push_str(&format!("## {}\n", cat));
        for op in ops {
            out.push_str(&format!("  {} — {}\n", op.name, op.description));
        }
        out.push('\n');
    }
    out.truncate(out.trim_end().len());
    out
}

/// Fuzzy-search the catalog by tokenizing the query and matching against
/// name (split on `_`), description, and category.
///
/// Results are sorted by score descending; when >5 results they are grouped
/// by category (alphabetical), losing strict global score order.
pub fn discover_search(query: &str) -> String {
    let tokens: Vec<String> = query
        .split(|c: char| c.is_whitespace() || c == '-' || c == '_')
        .map(|t| t.to_ascii_lowercase())
        .filter(|t| !t.is_empty())
        .collect();

    if tokens.is_empty() {
        return discover_all();
    }

    let mut scored: Vec<(usize, &Operation)> = CATALOG
        .iter()
        .filter_map(|op| {
            let score = match_score(op, &tokens);
            if score > 0 { Some((score, op)) } else { None }
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0));

    if scored.is_empty() {
        return format!("No operations found matching '{}'.", query);
    }

    // Group by category if >5 results, otherwise flat list.
    if scored.len() > 5 {
        let mut by_category: BTreeMap<&str, Vec<&Operation>> = BTreeMap::new();
        for (_, op) in &scored {
            by_category.entry(op.category).or_default().push(op);
        }
        let mut out = String::new();
        for (cat, ops) in &by_category {
            out.push_str(&format!("## {}\n", cat));
            for op in ops {
                out.push_str(&format!("  {} — {}\n", op.name, op.description));
            }
            out.push('\n');
        }
        out.truncate(out.trim_end().len());
        out
    } else {
        let mut out = String::new();
        for (_, op) in &scored {
            out.push_str(&format!("{} — {}\n", op.name, op.description));
            if !op.params.is_empty() {
                let param_names: Vec<&str> = op.params.iter().map(|p| p.name).collect();
                out.push_str(&format!("  params: {}\n", param_names.join(", ")));
            }
        }
        out.truncate(out.trim_end().len());
        out
    }
}

/// Score an operation against search tokens. Higher = better match.
fn match_score(op: &Operation, tokens: &[String]) -> usize {
    let name_parts: Vec<String> = op.name.split('_').map(|s| s.to_ascii_lowercase()).collect();
    let desc_lower = op.description.to_ascii_lowercase();
    let cat_lower = op.category.to_ascii_lowercase();

    let mut score = 0;
    for token in tokens {
        // Exact name-part match (e.g. "create" matches "create_agent")
        if name_parts.iter().any(|p| p == token) {
            score += 3;
        // Prefix match on name parts (e.g. "sess" matches "sessions")
        } else if name_parts.iter().any(|p| p.starts_with(token.as_str())) {
            score += 2;
        // Category match
        } else if cat_lower == token.as_str() || cat_lower.starts_with(token.as_str()) {
            score += 2;
        // Substring in description
        } else if desc_lower.contains(token.as_str()) {
            score += 1;
        }
    }
    score
}
