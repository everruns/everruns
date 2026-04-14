// API operations catalog — each entry becomes a ScriptedTool builtin
//
// Every Operation has a `name` used as the bash command name in the
// ScriptedTool interpreter, plus enough metadata to generate the ToolDef
// and the direct service callback.

use super::handlers;
use bashkit::{ScriptingToolSet, ToolArgs, ToolDef};
use serde_json::json;
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

/// Build a ScriptingToolSet with direct service calls for all catalog operations.
pub fn build_toolset(ctx: CatalogContext) -> ScriptingToolSet {
    let mut builder = ScriptingToolSet::builder("everruns")
        .short_description("Everruns API operations as bash builtins")
        .limits(
            bashkit::ExecutionLimits::new()
                .max_commands(500)
                .max_loop_iterations(5000)
                .max_function_depth(50)
                .max_input_bytes(500_000)
                .max_ast_depth(50)
                .parser_timeout(std::time::Duration::from_secs(3)),
        );

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

        tokio::task::block_in_place(|| {
            let handle = tokio::runtime::Handle::current();
            handle.block_on(async { (op.handler)(params, &ctx).await })
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
const CLIENT_FLAGS: &[&str] = &["help"];

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
    out
}

/// Check if a flag-like param is truthy (bool true or string "true").
fn is_flag_set(params: &serde_json::Value, key: &str) -> bool {
    params
        .get(key)
        .is_some_and(|v| v.as_bool().unwrap_or(false) || v.as_str().is_some_and(|s| s == "true"))
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
        description: "List all active agents. Use search for name search, include_archived=true to include archived. Supports pagination (limit/offset).",
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
        description: "List sessions. Filter by agent_id, search by title. Supports pagination (limit/offset).",
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
        description: "List harnesses (base environments for sessions).",
        params: &[Param {
            name: "include_archived",
            typ: "query",
            description: "Include archived (default: false)",
        }],
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
        description: "List all LLM models across all providers.",
        params: &[],
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
        description: "List configured LLM providers (OpenAI, Anthropic, etc.).",
        params: &[],
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
        description: "List registered MCP servers.",
        params: &[Param {
            name: "search",
            typ: "query",
            description: "Search by name",
        }],
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
        description: "List available capabilities (virtual bash, web fetch, MCP, etc.). Supports search and pagination (limit/offset).",
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
        description: "List registered skills.",
        params: &[Param {
            name: "include_archived",
            typ: "query",
            description: "Include archived",
        }],
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
        description: "List uploaded images. Supports pagination (limit/offset).",
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
        description: "List durable scheduled tasks.",
        params: &[],
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
        description: "List organizations for the current user.",
        params: &[],
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
        description: "List users in the current organization. Supports search filtering.",
        params: &[Param {
            name: "search",
            typ: "query",
            description: "Filter users by search term",
        }],
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
