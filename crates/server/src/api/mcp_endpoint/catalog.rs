// API operations catalog — each entry becomes a ScriptedTool builtin
//
// Every Operation has a `name` used as the bash command name in the
// ScriptedTool interpreter, plus enough metadata to generate the ToolDef
// and the HTTP callback.

use bashkit::{ScriptedTool, ToolArgs, ToolDef};
use serde_json::json;
use std::collections::BTreeMap;

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
        let mut wants_summary = false;

        if let Some(obj) = params.as_object() {
            for (key, value) in obj {
                // Intercept summary — handled client-side via apply_summary_filter,
                // never forwarded to the API.
                if key == "summary" {
                    wants_summary = value.as_bool().unwrap_or(false)
                        || value.as_str().is_some_and(|s| s == "true");
                    continue;
                }

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
                    if wants_summary {
                        Ok(apply_summary_filter(&body))
                    } else {
                        Ok(body)
                    }
                } else {
                    Err(format!("HTTP {status}: {body}"))
                }
            })
        })
    }
}

/// Summary fields kept in compact output mode.
const SUMMARY_FIELDS: &[&str] = &["id", "name", "description", "status"];

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
    },
    Operation {
        name: "list_agents",
        method: "GET",
        path: "/v1/agents",
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
                description: "Compact output: id, name, description, status only (default: false)",
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
                description: "New addressable name. Lowercase letters, numbers, and hyphens only.",
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
                description: "Compact output: id, name, description, status only (default: false)",
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
    // ── Budgets ─────────────────────────────────────────────────────────
    Operation {
        name: "create_budget",
        method: "POST",
        path: "/v1/budgets",
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
    },
    Operation {
        name: "list_budgets",
        method: "GET",
        path: "/v1/budgets",
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
    },
    Operation {
        name: "get_budget",
        method: "GET",
        path: "/v1/budgets/{budget_id}",
        category: "budgets",
        description: "Get a budget with current balance.",
        params: &[Param {
            name: "budget_id",
            typ: "path",
            description: "Budget ID",
        }],
    },
    Operation {
        name: "update_budget",
        method: "PATCH",
        path: "/v1/budgets/{budget_id}",
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
    },
    Operation {
        name: "delete_budget",
        method: "DELETE",
        path: "/v1/budgets/{budget_id}",
        category: "budgets",
        description: "Soft-delete a budget (sets status to disabled).",
        params: &[Param {
            name: "budget_id",
            typ: "path",
            description: "Budget ID",
        }],
    },
    Operation {
        name: "top_up_budget",
        method: "POST",
        path: "/v1/budgets/{budget_id}/top-up",
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
    },
    Operation {
        name: "check_budget",
        method: "GET",
        path: "/v1/budgets/{budget_id}/check",
        category: "budgets",
        description: "Check budget status and remaining balance.",
        params: &[Param {
            name: "budget_id",
            typ: "path",
            description: "Budget ID",
        }],
    },
    Operation {
        name: "list_session_budgets",
        method: "GET",
        path: "/v1/sessions/{session_id}/budgets",
        category: "budgets",
        description: "List all budgets for a session.",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
    },
    Operation {
        name: "check_session_budgets",
        method: "GET",
        path: "/v1/sessions/{session_id}/budget-check",
        category: "budgets",
        description: "Check all budgets for a session (currently includes session and agent scopes).",
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
                description: "Compact output: id, name, description, status only (default: false)",
            },
        ],
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
        description: "List all LLM models across all providers. Supports --summary for compact output.",
        params: &[Param {
            name: "summary",
            typ: "query",
            description: "Compact output: id, name, description, status only (default: false)",
        }],
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
        description: "List configured LLM providers (OpenAI, Anthropic, etc.). Supports --summary for compact output.",
        params: &[Param {
            name: "summary",
            typ: "query",
            description: "Compact output: id, name, description, status only (default: false)",
        }],
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
                description: "Compact output: id, name, description, status only (default: false)",
            },
        ],
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
                description: "Compact output: id, name, description, status only (default: false)",
            },
        ],
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
                description: "Compact output: id, name, description, status only (default: false)",
            },
        ],
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
                description: "Compact output: id, name, description, status only (default: false)",
            },
        ],
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
        description: "List durable scheduled tasks. Supports --summary for compact output.",
        params: &[Param {
            name: "summary",
            typ: "query",
            description: "Compact output: id, name, description, status only (default: false)",
        }],
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
        description: "List organizations for the current user. Supports --summary for compact output.",
        params: &[Param {
            name: "summary",
            typ: "query",
            description: "Compact output: id, name, description, status only (default: false)",
        }],
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
                description: "Compact output: id, name, description, status only (default: false)",
            },
        ],
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
/// Returns results sorted by match score (descending), formatted as plain text.
pub fn discover_search(query: &str) -> String {
    let tokens: Vec<String> = query
        .split_whitespace()
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
        } else if cat_lower == *token || cat_lower.starts_with(token.as_str()) {
            score += 2;
        // Substring in description
        } else if desc_lower.contains(token.as_str()) {
            score += 1;
        }
    }
    score
}
