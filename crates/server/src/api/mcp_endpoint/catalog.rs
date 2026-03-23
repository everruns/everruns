// API operations catalog for the MCP `discover` tool
//
// Static catalog of all Everruns API operations. Each entry has enough detail
// for an LLM to construct the correct curl command for the `execute` tool.
//
// Curl examples use $EVERRUNS_API_BASE and $EVERRUNS_API_KEY which are
// pre-configured environment variables in the execute tool's bashkit sandbox.

/// A single API operation in the catalog.
pub struct Operation {
    pub method: &'static str,
    pub path: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub params: &'static [Param],
    pub curl_example: &'static str,
}

/// A parameter for an API operation.
pub struct Param {
    pub name: &'static str,
    pub typ: &'static str,
    pub description: &'static str,
}

/// Search the catalog by query string and optional category.
pub fn search(query: &str, category: Option<&str>) -> Vec<&'static Operation> {
    let terms: Vec<&str> = query.split_whitespace().collect();

    CATALOG
        .iter()
        .filter(|op| {
            // Category filter
            if let Some(cat) = category
                && !op.category.eq_ignore_ascii_case(cat)
            {
                return false;
            }
            // All query terms must match somewhere (method, path, description, category)
            terms.iter().all(|term| {
                op.method.to_lowercase().contains(term)
                    || op.path.to_lowercase().contains(term)
                    || op.description.to_lowercase().contains(term)
                    || op.category.to_lowercase().contains(term)
            })
        })
        .collect()
}

// ============================================================================
// Catalog entries
// ============================================================================

static CATALOG: &[Operation] = &[
    // ── Agents ──────────────────────────────────────────────────────────
    Operation {
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
        curl_example: "curl -s -X POST -H 'Authorization: Bearer $EVERRUNS_API_KEY' -H 'Content-Type: application/json' -d '{\"name\": \"My Agent\", \"system_prompt\": \"You are helpful.\"}' $EVERRUNS_API_BASE/v1/agents",
    },
    Operation {
        method: "GET",
        path: "/v1/agents",
        category: "agents",
        description: "List all active agents. Use ?search= for name search, ?include_archived=true to include archived.",
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
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/agents",
    },
    Operation {
        method: "GET",
        path: "/v1/agents/{id}",
        category: "agents",
        description: "Get a single agent by ID.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Agent ID (format: agent_{32-hex})",
        }],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/agents/AGENT_ID",
    },
    Operation {
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
        curl_example: "curl -s -X PATCH -H 'Authorization: Bearer $EVERRUNS_API_KEY' -H 'Content-Type: application/json' -d '{\"name\": \"Updated Agent\"}' $EVERRUNS_API_BASE/v1/agents/AGENT_ID",
    },
    Operation {
        method: "DELETE",
        path: "/v1/agents/{id}",
        category: "agents",
        description: "Archive an agent (soft delete). Can be restored.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Agent ID",
        }],
        curl_example: "curl -s -X DELETE -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/agents/AGENT_ID",
    },
    Operation {
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
        curl_example: "curl -s -X PUT -H 'Authorization: Bearer $EVERRUNS_API_KEY' -H 'Content-Type: application/json' -d '{\"name\": \"My Agent\"}' $EVERRUNS_API_BASE/v1/agents/AGENT_ID",
    },
    Operation {
        method: "POST",
        path: "/v1/agents/{id}/copy",
        category: "agents",
        description: "Copy an agent with a new ID and '{name} (copy)' name.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Source agent ID",
        }],
        curl_example: "curl -s -X POST -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/agents/AGENT_ID/copy",
    },
    Operation {
        method: "GET",
        path: "/v1/agents/{id}/export",
        category: "agents",
        description: "Export agent definition as Markdown.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Agent ID",
        }],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/agents/AGENT_ID/export",
    },
    Operation {
        method: "POST",
        path: "/v1/agents/import",
        category: "agents",
        description: "Import agent from Markdown file content.",
        params: &[Param {
            name: "content",
            typ: "string",
            description: "Markdown content to import",
        }],
        curl_example: "curl -s -X POST -H 'Authorization: Bearer $EVERRUNS_API_KEY' -H 'Content-Type: application/json' -d '{\"content\": \"# Agent\\n...\"}' $EVERRUNS_API_BASE/v1/agents/import",
    },
    Operation {
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
        curl_example: "curl -s -X POST -H 'Authorization: Bearer $EVERRUNS_API_KEY' -H 'Content-Type: application/json' -d '{\"system_prompt\": \"You are helpful.\", \"capabilities\": []}' $EVERRUNS_API_BASE/v1/agents/preview",
    },
    // ── Sessions ────────────────────────────────────────────────────────
    Operation {
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
        curl_example: "curl -s -X POST -H 'Authorization: Bearer $EVERRUNS_API_KEY' -H 'Content-Type: application/json' -d '{\"agent_id\": \"AGENT_ID\", \"title\": \"My Session\"}' $EVERRUNS_API_BASE/v1/sessions",
    },
    Operation {
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
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/sessions",
    },
    Operation {
        method: "GET",
        path: "/v1/sessions/{session_id}",
        category: "sessions",
        description: "Get session details including status, agent, harness, and model.",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/sessions/SESSION_ID",
    },
    Operation {
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
        curl_example: "curl -s -X PATCH -H 'Authorization: Bearer $EVERRUNS_API_KEY' -H 'Content-Type: application/json' -d '{\"title\": \"New Title\"}' $EVERRUNS_API_BASE/v1/sessions/SESSION_ID",
    },
    Operation {
        method: "DELETE",
        path: "/v1/sessions/{session_id}",
        category: "sessions",
        description: "Delete a session.",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
        curl_example: "curl -s -X DELETE -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/sessions/SESSION_ID",
    },
    Operation {
        method: "POST",
        path: "/v1/sessions/{session_id}/cancel",
        category: "sessions",
        description: "Cancel the currently executing turn in a session.",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
        curl_example: "curl -s -X POST -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/sessions/SESSION_ID/cancel",
    },
    // ── Messages ────────────────────────────────────────────────────────
    Operation {
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
                name: "message.role",
                typ: "string",
                description: "Always 'user'",
            },
            Param {
                name: "message.content",
                typ: "array",
                description: "[{\"type\": \"text\", \"text\": \"...\"}]",
            },
        ],
        curl_example: "curl -s -X POST -H 'Authorization: Bearer $EVERRUNS_API_KEY' -H 'Content-Type: application/json' -d '{\"message\": {\"role\": \"user\", \"content\": [{\"type\": \"text\", \"text\": \"Hello\"}]}}' $EVERRUNS_API_BASE/v1/sessions/SESSION_ID/messages",
    },
    Operation {
        method: "GET",
        path: "/v1/sessions/{session_id}/messages",
        category: "messages",
        description: "List all messages in a session (user and agent messages).",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/sessions/SESSION_ID/messages",
    },
    // ── Events ──────────────────────────────────────────────────────────
    Operation {
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
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' '$EVERRUNS_API_BASE/v1/sessions/SESSION_ID/events?limit=20'",
    },
    Operation {
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
        curl_example: "curl -s -N -H 'Authorization: Bearer $EVERRUNS_API_KEY' '$EVERRUNS_API_BASE/v1/sessions/SESSION_ID/sse'",
    },
    // ── Harnesses ───────────────────────────────────────────────────────
    Operation {
        method: "GET",
        path: "/v1/harnesses",
        category: "harnesses",
        description: "List harnesses (base environments for sessions).",
        params: &[Param {
            name: "include_archived",
            typ: "query",
            description: "Include archived (default: false)",
        }],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/harnesses",
    },
    Operation {
        method: "GET",
        path: "/v1/harnesses/{id}",
        category: "harnesses",
        description: "Get a harness by ID.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Harness ID",
        }],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/harnesses/HARNESS_ID",
    },
    // ── LLM Models ──────────────────────────────────────────────────────
    Operation {
        method: "GET",
        path: "/v1/llm-models",
        category: "models",
        description: "List all LLM models across all providers.",
        params: &[],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/llm-models",
    },
    Operation {
        method: "GET",
        path: "/v1/llm-models/{id}",
        category: "models",
        description: "Get a single LLM model by ID.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Model ID",
        }],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/llm-models/MODEL_ID",
    },
    // ── LLM Providers ───────────────────────────────────────────────────
    Operation {
        method: "GET",
        path: "/v1/llm-providers",
        category: "providers",
        description: "List configured LLM providers (OpenAI, Anthropic, etc.).",
        params: &[],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/llm-providers",
    },
    Operation {
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
        curl_example: "curl -s -X POST -H 'Authorization: Bearer $EVERRUNS_API_KEY' -H 'Content-Type: application/json' -d '{\"name\": \"OpenAI\", \"provider_type\": \"openai\", \"api_key\": \"sk-...\"}' $EVERRUNS_API_BASE/v1/llm-providers",
    },
    // ── MCP Servers ─────────────────────────────────────────────────────
    Operation {
        method: "GET",
        path: "/v1/mcp-servers",
        category: "mcp_servers",
        description: "List registered MCP servers.",
        params: &[Param {
            name: "search",
            typ: "query",
            description: "Search by name",
        }],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/mcp-servers",
    },
    Operation {
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
        curl_example: "curl -s -X POST -H 'Authorization: Bearer $EVERRUNS_API_KEY' -H 'Content-Type: application/json' -d '{\"name\": \"my-mcp\", \"url\": \"https://example.com/mcp\"}' $EVERRUNS_API_BASE/v1/mcp-servers",
    },
    Operation {
        method: "GET",
        path: "/v1/mcp-servers/{id}",
        category: "mcp_servers",
        description: "Get an MCP server by ID.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "MCP server ID",
        }],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/mcp-servers/MCP_ID",
    },
    // ── Capabilities ────────────────────────────────────────────────────
    Operation {
        method: "GET",
        path: "/v1/capabilities",
        category: "capabilities",
        description: "List all available capabilities (virtual bash, web fetch, MCP, etc.).",
        params: &[],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/capabilities",
    },
    Operation {
        method: "GET",
        path: "/v1/capabilities/{id}",
        category: "capabilities",
        description: "Get capability details by ID.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Capability ID",
        }],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/capabilities/CAPABILITY_ID",
    },
    // ── Skills ──────────────────────────────────────────────────────────
    Operation {
        method: "GET",
        path: "/v1/skills",
        category: "skills",
        description: "List registered skills.",
        params: &[Param {
            name: "include_archived",
            typ: "query",
            description: "Include archived",
        }],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/skills",
    },
    Operation {
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
        curl_example: "curl -s -X POST -H 'Authorization: Bearer $EVERRUNS_API_KEY' -H 'Content-Type: application/json' -d '{\"name\": \"my-skill\", \"description\": \"...\", \"source_type\": \"inline\"}' $EVERRUNS_API_BASE/v1/skills",
    },
    // ── Images ──────────────────────────────────────────────────────────
    Operation {
        method: "GET",
        path: "/v1/images",
        category: "images",
        description: "List uploaded images.",
        params: &[],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/images",
    },
    Operation {
        method: "GET",
        path: "/v1/images/{id}",
        category: "images",
        description: "Get image data by ID.",
        params: &[Param {
            name: "id",
            typ: "path",
            description: "Image ID",
        }],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/images/IMAGE_ID",
    },
    // ── Schedules ───────────────────────────────────────────────────────
    Operation {
        method: "GET",
        path: "/v1/schedules",
        category: "schedules",
        description: "List durable scheduled tasks.",
        params: &[],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/schedules",
    },
    Operation {
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
        curl_example: "curl -s -X POST -H 'Authorization: Bearer $EVERRUNS_API_KEY' -H 'Content-Type: application/json' -d '{\"name\": \"daily-check\", \"cron\": \"0 9 * * *\", \"target\": {}}' $EVERRUNS_API_BASE/v1/schedules",
    },
    // ── Organizations ───────────────────────────────────────────────────
    Operation {
        method: "GET",
        path: "/v1/orgs",
        category: "organizations",
        description: "List organizations for the current user.",
        params: &[],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/orgs",
    },
    Operation {
        method: "GET",
        path: "/v1/orgs/{org}",
        category: "organizations",
        description: "Get organization details.",
        params: &[Param {
            name: "org",
            typ: "path",
            description: "Organization ID or slug",
        }],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/orgs/ORG_ID",
    },
    // ── Users ───────────────────────────────────────────────────────────
    Operation {
        method: "GET",
        path: "/v1/users",
        category: "users",
        description: "List users in the current organization.",
        params: &[],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/users",
    },
    // ── Session Files ───────────────────────────────────────────────────
    Operation {
        method: "GET",
        path: "/v1/sessions/{session_id}/files",
        category: "files",
        description: "Get the root directory listing of session files.",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/sessions/SESSION_ID/files",
    },
    Operation {
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
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/sessions/SESSION_ID/files/path/to/file",
    },
    // ── Session Databases ───────────────────────────────────────────────
    Operation {
        method: "GET",
        path: "/v1/sessions/{session_id}/databases",
        category: "databases",
        description: "List session-scoped SQL databases.",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/sessions/SESSION_ID/databases",
    },
    // ── Session Storage ─────────────────────────────────────────────────
    Operation {
        method: "GET",
        path: "/v1/sessions/{session_id}/storage/keys",
        category: "storage",
        description: "List key-value pairs in session storage.",
        params: &[Param {
            name: "session_id",
            typ: "path",
            description: "Session ID",
        }],
        curl_example: "curl -s -H 'Authorization: Bearer $EVERRUNS_API_KEY' $EVERRUNS_API_BASE/v1/sessions/SESSION_ID/storage/keys",
    },
    // ── Tool Results ────────────────────────────────────────────────────
    Operation {
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
        curl_example: "curl -s -X POST -H 'Authorization: Bearer $EVERRUNS_API_KEY' -H 'Content-Type: application/json' -d '{\"results\": [{\"tool_call_id\": \"...\", \"output\": \"...\"}]}' $EVERRUNS_API_BASE/v1/sessions/SESSION_ID/tool-results",
    },
    // ── Health ──────────────────────────────────────────────────────────
    Operation {
        method: "GET",
        path: "/health",
        category: "system",
        description: "Health check endpoint. Returns server version and runner mode.",
        params: &[],
        curl_example: "curl -s $EVERRUNS_API_BASE/health",
    },
];
