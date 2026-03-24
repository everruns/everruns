// API operations catalog — each entry becomes a ScriptedTool builtin
//
// Every Operation has a `name` used as the bash command name in the
// ScriptedTool interpreter, plus enough metadata to generate the ToolDef
// and the HTTP callback.

use bashkit::{ScriptedTool, ToolArgs, ToolDef};
use serde_json::json;

/// A single API operation in the catalog.
pub struct Operation {
    pub name: &'static str,
    pub method: &'static str,
    pub path: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub params: &'static [Param],
}

/// A parameter for an API operation.
pub struct Param {
    pub name: &'static str,
    pub typ: &'static str,
    pub description: &'static str,
}

/// Build a ScriptedTool with all catalog operations registered as builtins.
///
/// Each operation becomes a command callable from bash scripts.  The callback
/// for each command makes an HTTP request to the local API.
pub fn build_scripted_tool(api_base: &str, api_key: &str) -> ScriptedTool {
    let mut builder = ScriptedTool::builder("everruns")
        .short_description("Everruns API operations as bash builtins")
        .env("EVERRUNS_API_BASE", api_base)
        .env("EVERRUNS_API_KEY", api_key)
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
        let callback = make_http_callback(op.method, op.path, api_base, api_key);
        builder = builder.tool(def, callback);
    }

    builder.build()
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

/// Create an HTTP callback for an API operation.
///
/// Uses `tokio::task::block_in_place` to bridge sync callback → async reqwest.
fn make_http_callback(
    method: &'static str,
    path_template: &'static str,
    api_base: &str,
    api_key: &str,
) -> impl Fn(&ToolArgs) -> Result<String, String> + Send + Sync + 'static {
    let api_base = api_base.to_string();
    let api_key = api_key.to_string();

    move |args: &ToolArgs| {
        let params = &args.params;

        // Substitute path parameters into the URL template
        let mut path = path_template.to_string();
        let mut body_params = serde_json::Map::new();
        let mut query_parts = Vec::new();

        if let Some(obj) = params.as_object() {
            for (key, value) in obj {
                let placeholder = format!("{{{key}}}");
                if path.contains(&placeholder) {
                    // Path parameter — substitute into URL
                    let val_str = value.as_str().unwrap_or(&value.to_string()).to_string();
                    path = path.replace(&placeholder, &val_str);
                } else if is_query_param(path_template, key)
                    || method == "GET"
                    || method == "DELETE"
                {
                    // Query parameter
                    let val_str = value.as_str().unwrap_or(&value.to_string()).to_string();
                    query_parts.push(format!("{}={}", key, urlencoding::encode(&val_str)));
                } else {
                    // Body parameter
                    body_params.insert(key.clone(), value.clone());
                }
            }
        }

        let mut url = format!("{api_base}{path}");
        if !query_parts.is_empty() {
            url.push('?');
            url.push_str(&query_parts.join("&"));
        }

        // Make the HTTP request (sync bridge)
        let api_key = api_key.clone();
        tokio::task::block_in_place(|| {
            let handle = tokio::runtime::Handle::current();
            handle.block_on(async {
                let client = reqwest::Client::new();
                let mut req = match method {
                    "POST" => client.post(&url),
                    "PUT" => client.put(&url),
                    "PATCH" => client.patch(&url),
                    "DELETE" => client.delete(&url),
                    _ => client.get(&url),
                };

                req = req.header("Authorization", format!("Bearer {api_key}"));

                if !body_params.is_empty() {
                    req = req
                        .header("Content-Type", "application/json")
                        .json(&body_params);
                }

                let resp = req
                    .timeout(std::time::Duration::from_secs(30))
                    .send()
                    .await
                    .map_err(|e| format!("HTTP error: {e}"))?;

                let status = resp.status();
                let body = resp
                    .text()
                    .await
                    .map_err(|e| format!("Failed to read response: {e}"))?;

                if status.is_success() {
                    Ok(body)
                } else {
                    Err(format!("HTTP {status}: {body}"))
                }
            })
        })
    }
}

/// Check if a param name maps to a query-typed parameter in the catalog.
fn is_query_param(path_template: &str, param_name: &str) -> bool {
    CATALOG.iter().any(|op| {
        op.path == path_template
            && op
                .params
                .iter()
                .any(|p| p.name == param_name && p.typ == "query")
    })
}

// ============================================================================
// Catalog entries
// ============================================================================

pub static CATALOG: &[Operation] = &[
    // ── Agents ──────────────────────────────────────────────────────────
    Operation {
        name: "create_agent",
        method: "POST",
        path: "/v1/agents",
        category: "agents",
        description: "Create a new agent with a name, system prompt, and optional capabilities.",
        params: &[
            Param {
                name: "name",
                typ: "string",
                description: "Agent name (required)",
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
    },
    Operation {
        name: "list_agents",
        method: "GET",
        path: "/v1/agents",
        category: "agents",
        description: "List all active agents. Use search for name search, include_archived=true to include archived.",
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
    },
    Operation {
        name: "get_agent",
        method: "GET",
        path: "/v1/agents/{id}",
        category: "agents",
        description: "Get a single agent by ID.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Agent ID (format: agent_{32-hex})",
        }],
    },
    Operation {
        name: "update_agent",
        method: "PATCH",
        path: "/v1/agents/{id}",
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
                description: "New name",
            },
            Param {
                name: "system_prompt",
                typ: "string",
                description: "New system prompt",
            },
        ],
    },
    Operation {
        name: "delete_agent",
        method: "DELETE",
        path: "/v1/agents/{id}",
        category: "agents",
        description: "Archive an agent (soft delete). Can be restored.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Agent ID",
        }],
    },
    Operation {
        name: "upsert_agent",
        method: "PUT",
        path: "/v1/agents/{id}",
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
    },
    Operation {
        name: "copy_agent",
        method: "POST",
        path: "/v1/agents/{id}/copy",
        category: "agents",
        description: "Copy an agent with a new ID and '{name} (copy)' name.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Source agent ID",
        }],
    },
    Operation {
        name: "export_agent",
        method: "GET",
        path: "/v1/agents/{id}/export",
        category: "agents",
        description: "Export agent definition as Markdown.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Agent ID",
        }],
    },
    Operation {
        name: "import_agent",
        method: "POST",
        path: "/v1/agents/import",
        category: "agents",
        description: "Import agent from Markdown file content.",
        params: &[Param {
            name: "content",
            typ: "string",
            description: "Markdown content to import",
        }],
    },
    Operation {
        name: "preview_agent",
        method: "POST",
        path: "/v1/agents/preview",
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
    },
    // ── Sessions ────────────────────────────────────────────────────────
    Operation {
        name: "create_session",
        method: "POST",
        path: "/v1/sessions",
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
    },
    Operation {
        name: "list_sessions",
        method: "GET",
        path: "/v1/sessions",
        category: "sessions",
        description: "List sessions. Filter by agent_id, search by title.",
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
                description: "Pagination offset",
            },
            Param {
                name: "limit",
                typ: "query",
                description: "Page size (max 100)",
            },
        ],
    },
    Operation {
        name: "get_session",
        method: "GET",
        path: "/v1/sessions/{session_id}",
        category: "sessions",
        description: "Get session details including status, agent, harness, and model.",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
    },
    Operation {
        name: "update_session",
        method: "PATCH",
        path: "/v1/sessions/{session_id}",
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
    },
    Operation {
        name: "delete_session",
        method: "DELETE",
        path: "/v1/sessions/{session_id}",
        category: "sessions",
        description: "Delete a session.",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
    },
    Operation {
        name: "cancel_session",
        method: "POST",
        path: "/v1/sessions/{session_id}/cancel",
        category: "sessions",
        description: "Cancel the currently executing turn in a session.",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
    },
    // ── Messages ────────────────────────────────────────────────────────
    Operation {
        name: "create_message",
        method: "POST",
        path: "/v1/sessions/{session_id}/messages",
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
    },
    Operation {
        name: "list_messages",
        method: "GET",
        path: "/v1/sessions/{session_id}/messages",
        category: "messages",
        description: "List all messages in a session (user and agent messages).",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
    },
    // ── Events ──────────────────────────────────────────────────────────
    Operation {
        name: "list_events",
        method: "GET",
        path: "/v1/sessions/{session_id}/events",
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
    },
    Operation {
        name: "stream_sse",
        method: "GET",
        path: "/v1/sessions/{session_id}/sse",
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
    },
    // ── Harnesses ───────────────────────────────────────────────────────
    Operation {
        name: "list_harnesses",
        method: "GET",
        path: "/v1/harnesses",
        category: "harnesses",
        description: "List harnesses (base environments for sessions).",
        params: &[Param {
            name: "include_archived",
            typ: "query",
            description: "Include archived (default: false)",
        }],
    },
    Operation {
        name: "get_harness",
        method: "GET",
        path: "/v1/harnesses/{id}",
        category: "harnesses",
        description: "Get a harness by ID.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Harness ID",
        }],
    },
    // ── LLM Models ──────────────────────────────────────────────────────
    Operation {
        name: "list_models",
        method: "GET",
        path: "/v1/llm-models",
        category: "models",
        description: "List all LLM models across all providers.",
        params: &[],
    },
    Operation {
        name: "get_model",
        method: "GET",
        path: "/v1/llm-models/{id}",
        category: "models",
        description: "Get a single LLM model by ID.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Model ID",
        }],
    },
    // ── LLM Providers ───────────────────────────────────────────────────
    Operation {
        name: "list_providers",
        method: "GET",
        path: "/v1/llm-providers",
        category: "providers",
        description: "List configured LLM providers (OpenAI, Anthropic, etc.).",
        params: &[],
    },
    Operation {
        name: "create_provider",
        method: "POST",
        path: "/v1/llm-providers",
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
    },
    // ── MCP Servers ─────────────────────────────────────────────────────
    Operation {
        name: "list_mcp_servers",
        method: "GET",
        path: "/v1/mcp-servers",
        category: "mcp_servers",
        description: "List registered MCP servers.",
        params: &[Param {
            name: "search",
            typ: "query",
            description: "Search by name",
        }],
    },
    Operation {
        name: "create_mcp_server",
        method: "POST",
        path: "/v1/mcp-servers",
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
    },
    Operation {
        name: "get_mcp_server",
        method: "GET",
        path: "/v1/mcp-servers/{id}",
        category: "mcp_servers",
        description: "Get an MCP server by ID.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "MCP server ID",
        }],
    },
    // ── Capabilities ────────────────────────────────────────────────────
    Operation {
        name: "list_capabilities",
        method: "GET",
        path: "/v1/capabilities",
        category: "capabilities",
        description: "List all available capabilities (virtual bash, web fetch, MCP, etc.).",
        params: &[],
    },
    Operation {
        name: "get_capability",
        method: "GET",
        path: "/v1/capabilities/{id}",
        category: "capabilities",
        description: "Get capability details by ID.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Capability ID",
        }],
    },
    // ── Skills ──────────────────────────────────────────────────────────
    Operation {
        name: "list_skills",
        method: "GET",
        path: "/v1/skills",
        category: "skills",
        description: "List registered skills.",
        params: &[Param {
            name: "include_archived",
            typ: "query",
            description: "Include archived",
        }],
    },
    Operation {
        name: "create_skill",
        method: "POST",
        path: "/v1/skills",
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
    },
    // ── Images ──────────────────────────────────────────────────────────
    Operation {
        name: "list_images",
        method: "GET",
        path: "/v1/images",
        category: "images",
        description: "List uploaded images.",
        params: &[],
    },
    Operation {
        name: "get_image",
        method: "GET",
        path: "/v1/images/{id}",
        category: "images",
        description: "Get image data by ID.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Image ID",
        }],
    },
    // ── Schedules ───────────────────────────────────────────────────────
    Operation {
        name: "list_schedules",
        method: "GET",
        path: "/v1/schedules",
        category: "schedules",
        description: "List durable scheduled tasks.",
        params: &[],
    },
    Operation {
        name: "create_schedule",
        method: "POST",
        path: "/v1/schedules",
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
    },
    // ── Organizations ───────────────────────────────────────────────────
    Operation {
        name: "list_orgs",
        method: "GET",
        path: "/v1/orgs",
        category: "organizations",
        description: "List organizations for the current user.",
        params: &[],
    },
    Operation {
        name: "get_org",
        method: "GET",
        path: "/v1/orgs/{org}",
        category: "organizations",
        description: "Get organization details.",
        params: &[Param {
            name: "org",
            typ: "path",
            description: "Organization ID or slug",
        }],
    },
    // ── Users ───────────────────────────────────────────────────────────
    Operation {
        name: "list_users",
        method: "GET",
        path: "/v1/users",
        category: "users",
        description: "List users in the current organization.",
        params: &[],
    },
    // ── Session Files ───────────────────────────────────────────────────
    Operation {
        name: "list_session_files",
        method: "GET",
        path: "/v1/sessions/{session_id}/files",
        category: "files",
        description: "Get the root directory listing of session files.",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
    },
    Operation {
        name: "get_session_file",
        method: "GET",
        path: "/v1/sessions/{session_id}/files/{path}",
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
    },
    // ── Session Databases ───────────────────────────────────────────────
    Operation {
        name: "list_session_databases",
        method: "GET",
        path: "/v1/sessions/{session_id}/databases",
        category: "databases",
        description: "List session-scoped SQL databases.",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
    },
    // ── Session Storage ─────────────────────────────────────────────────
    Operation {
        name: "list_session_storage",
        method: "GET",
        path: "/v1/sessions/{session_id}/storage/keys",
        category: "storage",
        description: "List key-value pairs in session storage.",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
    },
    // ── Tool Results ────────────────────────────────────────────────────
    Operation {
        name: "submit_tool_results",
        method: "POST",
        path: "/v1/sessions/{session_id}/tool-results",
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
    },
    // ── Health ──────────────────────────────────────────────────────────
    Operation {
        name: "health_check",
        method: "GET",
        path: "/health",
        category: "system",
        description: "Health check endpoint. Returns server version and runner mode.",
        params: &[],
    },
];
