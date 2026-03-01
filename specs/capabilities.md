# Capabilities Specification

## Abstract

Capabilities are modular functionality units that extend Agent behavior. A Capability can contribute additions to the system prompt, provide tools, and modify execution behavior. Users can enable multiple capabilities on an Agent to compose functionality.

## Requirements

### Concept

A Capability is an abstraction that defines added functionality for an Agent:

1. **System Prompt Contribution**: Text prepended to the agent's system prompt
2. **Tool Provision**: Tools made available to the agent during execution
3. **Behavior Modification**: Influence on tool invocation and execution (future)

### Architecture

Capabilities are defined in **everruns-core** and resolved at the **API layer**:

```
┌─────────────────────────────────────────────────────────┐
│                     everruns-core                        │
│  ┌─────────────────────────────────────────────────┐   │
│  │ CapabilityRegistry + Capability trait impls     │   │
│  │ (single source of truth for capability defs)   │   │
│  └─────────────────────────────────────────────────┘   │
└───────────────────────────┬─────────────────────────────┘
                            ↓
┌─────────────────────────────────────────────────────────┐
│                     API / Service Layer                  │
│  ┌─────────────────────────────────────────────────┐   │
│  │ CapabilityService (uses core registry directly) │   │
│  │ Capability::from_core() converts to DTOs        │   │
│  └─────────────────────────────────────────────────┘   │
└───────────────────────────┬─────────────────────────────┘
                            ↓
┌─────────────────────────────────────────────────────────┐
│                      Agent Loop                          │
│        (receives fully-configured RuntimeAgent)           │
└─────────────────────────────────────────────────────────┘
```

- Capabilities are defined in **everruns-core** (trait implementations)
- The API layer uses the core registry and converts to DTOs for responses
- The Agent Loop remains focused on execution
- RuntimeAgent is built with merged system prompt and tools from capabilities

### Data Model

#### Capability (Public DTO)

| Field | Type | Description |
|-------|------|-------------|
| `id` | CapabilityId | Unique string identifier |
| `name` | string | Display name |
| `description` | string | Description of functionality (supports markdown) |
| `status` | CapabilityStatus | Availability status |
| `icon` | string? | Icon name for UI rendering |
| `category` | string? | Category for grouping in UI |
| `is_mcp` | boolean | True if this is an MCP virtual capability |
| `dependencies` | string[] | IDs of capabilities this capability depends on |
| `features` | string[] | UI feature strings this capability contributes to |
| `risk_level` | RiskLevel | Risk classification: `low` (default, omitted in JSON), `medium`, `high` |

##### Description Markdown Support

Capability descriptions support GitHub Flavored Markdown including:

- Basic formatting: **bold**, *italic*, `code`, [links](url)
- Lists (ordered and unordered)
- Code blocks with syntax highlighting
- Tables
- GitHub-style alerts for callouts:

```markdown
> [!NOTE]
> Highlights information that users should take into account.

> [!TIP]
> Optional information to help a user be more successful.

> [!IMPORTANT]
> Crucial information necessary for users to succeed.

> [!WARNING]
> Critical content demanding immediate user attention due to potential risks.

> [!CAUTION]
> Negative potential consequences of an action.
```

Example capability description with alerts:

```
Fetch content from URLs and convert HTML to markdown.

> [!TIP]
> Use `as_markdown: true` for better readability when fetching HTML pages.

> [!WARNING]
> Binary content (images, PDFs) cannot be fetched as text. Only metadata is returned.
```

#### CapabilityId (String wrapper)

Capability IDs are string-based for extensibility. New capabilities can be added without database migrations. See `crates/core/src/capability_types.rs` for `CapabilityId`, `CapabilityStatus`, and `AgentCapabilityConfig` types.

##### ID Format

| Type | Format | Example |
|------|--------|---------|
| Built-in | `snake_case` identifier | `current_time`, `web_fetch` |
| MCP | `mcp:{server_uuid}` | `mcp:01933b5a-0000-7000-8000-000000000501` |

**Built-in IDs** use `snake_case` naming and are validated against the `CapabilityRegistry`. **MCP IDs** use the `mcp:` prefix followed by the MCP server's UUID. See `specs/mcp-servers.md` for details.

For the full list of built-in capability IDs, see `crates/core/src/capabilities/mod.rs` (registry initialization).

#### AgentCapabilityConfig (Per-agent configuration)

Associates a capability with an agent via `ref` (CapabilityId) + optional `config` (JSON). The `config` field is capability-specific and passed during execution, allowing per-agent behavior.

**Import compatibility:** For backward compatibility, import accepts both `{ "ref": "id", "config": {} }` and plain `"id"` formats.

#### Capability Trait (everruns-core)

See `crates/core/src/capabilities/mod.rs` for the `Capability` trait and `CapabilityRegistry`. Key trait methods: `id()`, `name()`, `description()`, `status()`, `system_prompt_contribution()`, `tools()`, `mounts()`, `dependencies()`, `features()`, `message_filter_provider()`.

##### System Prompt Methods

- **`system_prompt_addition()`** — Sync, returns static `&str`. Used by sync collection path.
- **`system_prompt_contribution(ctx)`** — Async, receives `SystemPromptContext` with session filesystem access. Default wraps `system_prompt_addition()` in XML tags. Capabilities needing dynamic content (e.g., `agent_instructions`, `skills`) override to read from session filesystem.

##### Collection Functions

- **`collect_capabilities()`** — Sync. Collects tools, mounts, static prompts.
- **`collect_capabilities_async(ctx)`** — Async. Calls `system_prompt_contribution()` for dynamic prompts.
- **`RuntimeAgentBuilder::with_capabilities_async(ctx)`** — Async builder method.

**Note**: `message_filter_provider()` returns an optional filter that modifies message retrieval. See [Message Filters](#message-filters).

### Capability Dependencies

Capabilities can declare dependencies on other capabilities. When a capability is selected, its dependencies are automatically resolved and applied.

#### How Dependencies Work

1. **Declaration**: Capabilities implement `dependencies()` returning a list of capability IDs they depend on
2. **Resolution**: When capabilities are applied, dependencies are resolved recursively
3. **Ordering**: Dependencies are always applied before dependents (topological order)
4. **Storage**: Only user-selected capabilities are stored in the database; dependencies are resolved at runtime
5. **UI Enforcement**: Users cannot remove a capability that is required by other selected capabilities

#### Dependency Resolution Rules

- Dependencies are resolved depth-first
- Circular dependencies are detected and return an error
- Maximum of 100 resolved capabilities to prevent resource exhaustion
- Unknown capabilities in dependency chains are silently skipped

#### Example

When `sample_data` is selected (it depends on `session_file_system`):
1. `session_file_system` is added as a dependency (if not already selected)
2. `session_file_system` tools and system prompt are applied first
3. `sample_data` mounts and system prompt are applied second
4. User cannot remove `session_file_system` while `sample_data` is selected

#### Dependency Resolution API

See `crates/core/src/capabilities/mod.rs` for `resolve_dependencies()` and `ResolvedCapabilities`.

#### Built-in Capabilities with Dependencies

| Capability | Depends On | Features |
|------------|-----------|----------|
| `sample_data` | `session_file_system` | `file_system` |
| `skills` | `session_file_system` | *(none)* |
| `virtual_bash` | `session_file_system` | `file_system` |

### Capability Features

Capabilities declare UI features they contribute to via `features()`. Features are open-ended strings indicating what user-facing functionality a capability enables. The Session DTO aggregates features from all active capabilities (harness + agent + session-level) at read time, and the UI uses the aggregated set to decide which tabs/sections to render.

#### How Features Work

1. **Declaration**: Capabilities implement `features()` returning a list of feature strings
2. **Aggregation**: `compute_features()` resolves dependencies and collects features from all resolved capabilities
3. **Deduplication**: Features are deduplicated — multiple capabilities contributing the same feature produce one entry
4. **Session DTO**: The `Session.features` field is computed at read time (not stored in the database)
5. **UI Rendering**: The UI conditionally renders tabs based on the feature set (e.g., Workspace tab only appears when `"file_system"` is present)

#### Built-in Feature Mapping

| Capability | Features |
|------------|----------|
| `session_file_system` | `file_system` |
| `virtual_bash` | `file_system` |
| `sample_data` | `file_system` |
| `session_storage` | `secrets`, `key_value` |
| `session_schedule` | `schedules` |
| `session_sql_database` | `sql_database` |

MCP and Skill virtual capabilities currently declare no features.

#### Known Feature Strings

| Feature | UI Element | Description |
|---------|------------|-------------|
| `file_system` | Workspace tab | Session has file system access |
| `secrets` | Storage tab | Session can store encrypted secrets |
| `key_value` | Storage tab | Session can store key/value pairs |
| `schedules` | Schedules tab | Session can create/manage schedules |
| `sql_database` | *(reserved)* | Session has SQL database access |

#### compute_features()

See `crates/core/src/capabilities/mod.rs` for `compute_features()` — resolves dependencies, collects features, deduplicates.

### Risk Levels (TM-AGENT-005)

Each capability declares a `RiskLevel` via the `Capability` trait. The API enforces that assigning high-risk capabilities to an agent requires `OrgRole::Admin`.

| Level | Description | Enforcement |
|-------|-------------|-------------|
| `low` | Default. No special requirements. | Any org member |
| `medium` | Logged but allowed for org members. | Any org member |
| `high` | Can execute arbitrary code or access external resources. | Requires Admin role |

High-risk built-in capabilities: `virtual_bash`, `web_fetch`, `docker_container`, `daytona`, `codesandbox`.

See `crates/core/src/capabilities/mod.rs` for the `RiskLevel` enum and `crates/server/src/api/agents.rs` for `require_admin_for_high_risk()`.

### Built-in Capabilities

For the full list of built-in capabilities with their tools, parameters, and system prompts, see `crates/core/src/capabilities/` (each capability has its own module).

#### FileSystem

- **ID**: `session_file_system`
- **Purpose**: Tools to access and manipulate files in the session workspace (`/workspace`)
- **Tools**: `read_file`, `write_file`, `list_directory`, `grep_files`, `delete_file`, `stat_file`
- **Workspace Mount**: All paths relative to `/workspace`, normalized internally for virtual_bash integration

##### Design Decision: Context-Aware Tools

FileSystem tools require session context to access the isolated virtual filesystem. Each tool implements `requires_context() -> true` and uses `execute_with_context()` instead of the standard `execute()`. The `ToolContext` provides:
- `session_id`: The session whose filesystem to access
- `file_store`: A `SessionFileStore` trait implementation for file operations

##### Design Decision: Session Isolation

Each session has its own isolated filesystem stored in PostgreSQL. Files are session-scoped and cannot be accessed across sessions. This provides security isolation and clean separation between conversations.

##### Design Decision: Auto-Create Parents

When writing a file like `/a/b/c.txt`, parent directories `/a` and `/a/b` are automatically created if they don't exist. This follows common filesystem patterns and reduces tool call overhead.

#### VirtualBash

- **ID**: `virtual_bash`
- **Purpose**: Sandboxed bash shell execution for safe code execution and file manipulation
- **Dependencies**: `session_file_system` (operates on same workspace)
- **Tools**: `bash` (execute command with optional working_dir and timeout)
- **Workspace Mount**: Session files at `/workspace`, shared with FileSystem tools

##### Design Decision: bashkit Library

This capability uses the [bashkit](https://github.com/everruns/bashkit) library for sandboxed bash execution. bashkit provides:
- WASM-like execution isolation (no system access)
- Custom FileSystem trait implementation for session file access
- Built-in shell commands (cd, ls, cat, echo, etc.)
- Resource limits to prevent infinite loops

##### Design Decision: SessionFileSystemAdapter

The `SessionFileSystemAdapter` implements bashkit's `FileSystem` trait, bridging bash file operations to the session file store. This enables:
- **Live file visibility**: Files written by other tools during bash execution are immediately visible
- **No sync overhead**: Eliminates pre/post execution sync of entire filesystem
- **Memory efficiency**: Files read on-demand instead of loading all into memory
- **Consistency**: Single source of truth for file state

##### Design Decision: Path Translation

Bash operates with `/workspace` as the root. The adapter translates paths:
- `/workspace/foo.txt` → `/foo.txt` (storage path)
- Paths outside `/workspace` return errors

This ensures bash and FileSystem capability share the same file namespace.

#### SessionStorage

- **ID**: `session_storage`
- **Purpose**: Session-scoped key/value storage and encrypted secret storage
- **Tools**: `kv_store` (set/get/delete/list), `secret_store` (encrypted, same ops)

##### Design Decision: Consolidated Tools

Two tools consolidate 8 operations (4 per storage type) to reduce tool count:
- `kv_store` with `operation` parameter (set/get/delete/list) for key/value storage
- `secret_store` with `operation` parameter (set/get/delete/list) for encrypted secrets

##### Design Decision: Key/Value vs Secrets

Two storage types are provided:
- **Key/Value storage**: For general data like state, preferences, or intermediate results. Stored as plain text in the database.
- **Secret storage**: For sensitive data like API keys, tokens, or credentials. Values are encrypted at rest using AES-256-GCM envelope encryption.

##### Design Decision: Session Isolation

Like the FileSystem capability, storage is session-scoped. Keys and secrets cannot be accessed across sessions.

##### Design Decision: Upsert Semantics

Both set operations use upsert semantics - storing with an existing key/name will overwrite the previous value.

##### Design Decision: Encryption Requirement for Secrets

Secret operations require the `SECRETS_ENCRYPTION_KEY` environment variable to be configured. If encryption is not configured, secret operations will return an error. Key/value operations work without encryption.

#### WebFetch

- **ID**: `web_fetch`
- **Purpose**: Fetch content from URLs and convert HTML to markdown or plain text
- **Tools**: `web_fetch` (url, method, as_markdown, as_text)
- **Timeouts**: First byte 1s, body 30s (partial content on timeout)

##### Design Decision: System Prompt from fetchkit

This capability uses the `TOOL_LLMTXT` constant from fetchkit as its system prompt addition. This provides comprehensive LLM-optimized documentation including:
- Capability overview
- Input parameters with descriptions
- Output fields with explanations
- Usage examples
- Error handling guidance

##### Design Decision: Binary Content Metadata

Binary content (images, PDFs, audio, video, etc.) cannot be fetched as textual content. However, instead of returning a tool error, the response includes available metadata (content type, size, filename, last modified) along with an error message explaining binary content is not supported. This allows agents to understand what the resource is even though they cannot access its content.

##### Design Decision: fetchkit Library

The capability uses the [fetchkit](https://github.com/everruns/fetchkit) library (v0.1.2) for all HTTP operations and HTML conversion. This provides:
- Consistent behavior across deployments
- Built-in HTML to markdown/text conversion optimized for LLM consumption
- Automatic binary content detection
- Timeout management with partial content on body timeout
- Excessive newline filtering
- **SSRF protection** via `DnsPolicy::block_private_ips()` (default): resolve-then-check validates resolved IPs against blocked ranges (loopback, RFC1918, link-local/cloud metadata, CGNAT, multicast) with DNS pinning to prevent rebinding attacks

#### StatelessTodoList

- **ID**: `stateless_todo_list`
- **Purpose**: Structured task lists for tracking multi-step work progress
- **Tools**: `write_todos` (replaces entire todo list each call; state in conversation history)

##### Design Decision: Stateless Implementation

This capability is intentionally **stateless** - it does not persist todos to a separate database table. State is maintained through conversation history (message storage).

###### Why Stateless?

This follows the same pattern as Claude Code's TodoWrite tool:
- Each `write_todos` call receives and returns the **complete** todo list
- The LLM remembers todos by reading previous tool calls from conversation history
- No separate storage layer needed - simpler implementation

###### Alternative Approaches (Research)

**LangChain DeepAgents TodoListMiddleware**:
- Uses dedicated `todos` state channel (not message history)
- Thread-scoped lifecycle with subagent isolation
- Known issue: context tokens grow quickly (proposed `auto_clean_context` flag)
- Reference: https://deepwiki.com/langchain-ai/deepagents/2.4-state-management

**OpenAI Codex CLI update_plan**:
- Tool named `update_plan` with explanation + plan items
- Maintains plan history across resumed runs
- Supports "compacting conversation state" for longer sessions
- Reference: https://github.com/openai/codex

###### Trade-offs

| Approach | Pros | Cons |
|----------|------|------|
| Stateless (current) | Simple, no DB changes | Context grows with messages |
| State channel | Efficient context | Complex middleware needed |
| DB persistence | Survives context loss | Requires schema changes |

###### Future Improvements

Consider adding context compaction (prune old `write_todos` calls) if context growth becomes an issue in long-running sessions.

##### When to Use StatelessTodoList

The system prompt instructs agents to use task management when:
1. **Complex multi-step tasks** - Tasks requiring 3 or more distinct steps
2. **User provides multiple tasks** - When users give a list of things to do
3. **Non-trivial work** - Tasks requiring careful planning
4. **After receiving new instructions** - Capture requirements immediately
5. **When starting work** - Mark a task as `in_progress` BEFORE beginning
6. **After completing work** - Mark task as `completed` and add follow-up tasks

##### Best Practices

1. **One task in progress** - Exactly one task should be `in_progress` at a time
2. **Update immediately** - Mark tasks completed as soon as done, don't batch
3. **Replace entire list** - Each `write_todos` call replaces the full list
4. **Completion criteria** - Only mark `completed` when fully done (tests pass, no errors)

#### SkillsDiscovery

- **ID**: `skills`
- **Purpose**: Discover and activate skills from `/.agents/skills/` in session VFS
- **Dependencies**: `session_file_system`
- **Tools**: `list_skills` (scan for SKILL.md files), `activate_skill` (load instructions by name)

##### Design Decision: VFS-Based Discovery

This is the built-in skills discovery mechanism. It does NOT ship with any skills — users upload SKILL.md files to `/.agents/skills/{name}/SKILL.md` in the session filesystem. The agent discovers them at runtime via `list_skills` and activates them on demand via `activate_skill`.

This complements registry-based skills (`skill:{uuid}` capabilities) which are database-stored and org-wide. VFS-discovered skills are per-session and project-specific.

##### Design Decision: Progressive Disclosure

Following the agentskills.io specification:
1. **Discovery**: `list_skills` returns only names and descriptions (~100 tokens per skill)
2. **Activation**: `activate_skill` loads full SKILL.md instructions (<5000 tokens recommended)
3. **Resources**: Bundled files listed in activation response, accessible via existing session filesystem tools

### MCP Virtual Capabilities

MCP servers (see `specs/mcp-servers.md`) are integrated as "virtual capabilities" alongside built-in capabilities. This allows agents to use MCP tools through the same capability selection UI.

#### Capability ID Format

MCP capabilities use a prefixed ID format:
```
mcp:{server_uuid}
```

Example: `mcp:01933b5a-0000-7000-8000-000000000501`

The UUID portion is the MCP server's unique identifier (UUID v7).

#### How MCP Capabilities Work

1. **Discovery**: When listing capabilities, active MCP servers are included with their cached tools
2. **Selection**: Agents can select MCP capabilities just like built-in capabilities
3. **Tool Execution**: MCP tools are prefixed with `mcp_{server_name}__` (double underscore) to avoid conflicts
4. **UI Badge**: MCP capabilities display an "MCP" badge in the capability selector

#### MCP Tool Name Format

MCP tools use a double-underscore separator to distinguish server name from tool name:

```
mcp_{server_name}__{tool_name}
```

**Why double underscore?** Server names can contain underscores (e.g., `microsoft_learn`), so a single underscore would be ambiguous. The double underscore provides unambiguous parsing.

**Examples:**
| Server | Tool | Full Tool Name |
|--------|------|----------------|
| `github` | `search_repos` | `mcp_github__search_repos` |
| `microsoft_learn` | `search` | `mcp_microsoft_learn__search` |
| `atlassian_jira` | `create_issue` | `mcp_atlassian_jira__create_issue` |

#### MCP Capability Properties

When an MCP server is converted to a capability:

| Property | Source |
|----------|--------|
| `id` | `mcp:{server.id}` |
| `name` | Server name (display name) |
| `description` | Server description |
| `is_mcp` | `true` |
| `icon` | `plug` (hardcoded) |
| `category` | `MCP Servers` |
| `status` | `available` if server active, else excluded |

#### Key Differences from Built-in Capabilities

| Aspect | Built-in | MCP |
|--------|----------|-----|
| Source | Rust code | Remote server |
| ID format | `snake_case` | `mcp:{uuid}` |
| Tool prefix | None | `mcp_{server}__` |
| Tool discovery | Compile-time | Runtime (cached, 24h TTL) |
| Execution | In-process | HTTP JSON-RPC |
| Availability | Always | Depends on server status |
| System prompt | Optional per-capability | None |
| Dependencies | Supported | Not supported |

See `specs/mcp-servers.md` for full MCP integration details including:
- Server configuration and authentication
- Tool discovery protocol
- Tool execution flow
- Error handling

### Capability Application Flow

When a session executes:

1. **Load Agent**: Fetch agent configuration from database
2. **Load Session**: Fetch session configuration including session-level capabilities
3. **Fetch Agent Capabilities**: Get agent's enabled capabilities via `get_agent_capabilities(agent_id)`
4. **Resolve Capabilities**: For each capability (ordered by `position`):
   - Look up `InternalCapability` from registry by string ID
   - Call `system_prompt_contribution(ctx)` (async) for dynamic prompt content
   - Collect `tools` definitions
5. **Apply Session Capabilities**: Session-level capabilities are applied **after** agent capabilities (additive):
   - Session capabilities extend the agent's tools
   - Session capability system prompts are prepended after agent capability prompts
   - This allows temporarily extending capabilities without modifying the agent
6. **Build RuntimeAgent**:
   - System prompt = session cap additions + agent cap additions + agent's base prompt
   - Each section is wrapped in XML tags for clear boundaries (see `specs/xml-prompt-formatting.md`):
     - AGENTS.md content: `<agent-instructions source="AGENTS.md">`
     - Each capability's addition: `<capability id="{cap_id}">`
     - Base prompt: `<system-prompt>` (only when capabilities contribute prompt additions)
   - Tools = merged tool list from agent capabilities + session capabilities
7. **Execute**: Run Agent Loop with fully configured RuntimeAgent

### Session Capabilities

Sessions can have their own capabilities that are additive to the agent's capabilities. This enables:
- Temporarily extending an agent's capabilities for specific sessions
- Per-session customization without modifying the agent configuration
- Testing new capabilities before adding them to the agent

Session capabilities use the same format as agent capabilities:

```json
{
  "capabilities": [
    { "ref": "current_time" },
    { "ref": "web_fetch", "config": { "timeout_ms": 30000 } }
  ]
}
```

Session capabilities are stored in the `sessions.capabilities` column (JSONB) and applied at runtime via `RuntimeAgentBuilder::with_capabilities()` after agent capabilities are applied.

### API Endpoints

Capabilities are managed as part of the agent resource. When creating or updating an agent, you can specify the capabilities to enable. The agent response includes the list of enabled capabilities.

All endpoints are organization-scoped.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/capabilities` | List all available capabilities |
| GET | `/v1/capabilities/{capability_id}` | Get capability details |

Agent capabilities are managed through the agents API:
- `POST /v1/agents` - Create agent with capabilities
- `PATCH /v1/agents/{id}` - Update agent capabilities
- `GET /v1/agents/{id}` - Get agent (includes capabilities)

Capabilities are managed via the agents API (`POST /v1/agents`, `PATCH /v1/agents/{id}`). The capabilities array order determines priority (earlier = first in system prompt). See `specs/apis.md` and the OpenAPI spec for request/response formats.

### Database Schema

See `crates/server/migrations/001_base_schema.sql` for the `agent_capabilities` table DDL.

**Note**: The `capability_id` column has no CHECK constraint — validation is at the application layer via `CapabilityRegistry`. This allows adding capabilities without database migrations.

### Design Decisions

| Question | Decision |
|----------|----------|
| Where are capabilities defined? | In-memory registry (not database) |
| How are they applied? | Resolved at API layer, merged into RuntimeAgent |
| Order of application? | By `position` field (lower = earlier) |
| Can capabilities conflict? | Currently no conflict resolution; later capabilities add to earlier ones |
| Can users create custom capabilities? | Not in current version (built-in only) |
| How are new capabilities added? | Implement `Capability` trait with unique `id()` and register in `CapabilityRegistry` - no database changes or central ID definitions needed |

### Adding New Capabilities

1. Implement the `Capability` trait (see `crates/core/src/capabilities/mod.rs` for trait definition)
2. Register in `CapabilityRegistry::with_builtins()` (same file)
3. Add tool implementations if needed (implement `Tool` trait from `crates/core/src/tools.rs`)
4. No database migration required — capability ID validated at runtime

### Capability Mount Points

Capabilities can declare mount points to populate files and directories in the session filesystem when a session is created. This allows capabilities to provide sample data, documentation, configuration files, or other resources that agents can access during execution.

#### Mount Point Data Model

See `crates/core/src/capability_types.rs` for `MountPoint`, `MountAccess`, `MountSource`, and `MountEntry` types.

Key concepts:
- `MountAccess`: `ReadOnly` or `ReadWrite`
- `MountSource`: `InlineFile` (content + encoding) or `InlineDirectory` (HashMap of entries)
- Capabilities declare mounts via the `mounts()` trait method

#### Mount Application Flow

1. Session is created via POST `/v1/sessions` (with `agent_id` in body)
2. SessionService fetches agent's capabilities
3. Mounts are collected from all enabled capabilities
4. Files and directories are created in session filesystem
5. Files from readonly mounts are marked `is_readonly = true`

#### Built-in Capabilities with Mounts

See `crates/core/src/capabilities/sample_data.rs` for a concrete example of `mounts()` implementation.

##### SampleData

- **Status**: Available
- **ID**: `sample_data`
- **Purpose**: Demonstrates capability mounting with sample data files
- **Mounts**:
  - `/samples/users.json` - Sample JSON user data (readonly)
  - `/samples/config.yaml` - Sample YAML configuration (readonly)
  - `/samples/README.md` - Documentation about sample files (readonly)
- **Icon**: "database"
- **Category**: "Data"

#### PlatformManagement

- **Status**: Available
- **ID**: `platform_management`
- **Purpose**: Programmatic management of Everruns entities (harnesses, agents, sessions) via tool calls
- **System Prompt**: Describes available management tools and common workflows
- **Tools**:
  - `manage_harnesses` - Harness CRUD operations
    - Parameters:
      - `operation`: enum (list, get, create, update, delete, copy) - The operation to perform
      - `harness_id`: string - Required for get, update, delete, copy
      - `name`: string - Required for create, optional for update/copy
      - `system_prompt`: string - Required for create, optional for update
      - `description`: string - Optional for create/update
      - `capabilities`: string[] - Optional for create
    - Returns: Harness data with `ui_link` for each harness
    - Policy: Auto
  - `manage_agents` - Agent CRUD operations
    - Parameters:
      - `operation`: enum (list, get, create, update, delete) - The operation to perform
      - `agent_id`: string - Required for get, update, delete
      - `name`: string - Required for create, optional for update
      - `system_prompt`: string - Required for create, optional for update
      - `description`: string - Optional for create/update
      - `capabilities`: string[] - Optional for create
    - Returns: Agent data with `ui_link` for each agent
    - Policy: Auto
  - `manage_sessions` - Session CRUD operations
    - Parameters:
      - `operation`: enum (list, get, create, delete) - The operation to perform
      - `session_id`: string - Required for get, delete
      - `harness_id`: string - Required for create
      - `agent_id`: string - Optional for create/list
      - `title`: string - Optional for create
      - `limit`: integer - Optional for list
    - Returns: Session data with `ui_link` for each session
    - Policy: Auto
  - `session_interact` - Session messaging and turn management
    - Parameters:
      - `operation`: enum (send_message, get_messages, wait_for_idle) - The operation to perform
      - `session_id`: string - Required for all operations
      - `content`: string - Required for send_message
      - `limit`: integer - Optional for get_messages (default: 10)
      - `timeout_secs`: integer - Optional for wait_for_idle (default: 300)
    - Returns: Message data or status
    - Policy: Auto
- **Icon**: "settings"
- **Category**: "Management"

##### Design Decision: Context-Aware Tools

All platform management tools require session context to access the `PlatformStore`. Each tool implements `requires_context() -> true` and uses `execute_with_context()`. The `ToolContext` provides a `platform_store` field with the `PlatformStore` implementation.

##### Design Decision: UI Links

All tool results include `ui_link` fields pointing to the relevant UI page (e.g., `/harnesses/{id}`, `/agents/{id}`, `/chat?session={id}`). This lets agents direct users to the web interface for visual management.

##### Design Decision: PlatformStore Trait

The `PlatformStore` trait (in `everruns-core`) defines the org-scoped management API. `DirectPlatformStore` (in `everruns-server`) implements it using the existing `StorageBackend` and `SessionService`. This keeps the capability in core while the implementation uses server-layer storage.

### Experimental Capabilities

Experimental capabilities are available in development environments only (`DeploymentGrade::Dev`). They may change significantly or be removed.

#### DockerContainer

- **ID**: `docker_container` (Dev only, integration plugin)
- **Crate**: `integrations/docker/` (auto-registered via `inventory`, force-linked — see [architecture.md](architecture.md#integration-plugin-force-linking))
- **Purpose**: Run commands and manage files in a session-scoped Docker container
- **Tools**: `docker_exec`, `docker_read_file`, `docker_write_file`, `docker_logs`, `docker_stop`
- **Container Lifecycle**: Lazily started on first use, persists for session, named `everruns-{session_id}`

##### Design Decision: Session-Scoped Containers

Each session gets its own isolated Docker container named `everruns-{session_id}`. This provides:
- Isolation between sessions
- Clean state on session creation
- Automatic cleanup capability via `docker_stop`

##### Design Decision: Lazy Container Start

Containers are not started until the first tool use (`docker_exec`, `docker_read_file`, or `docker_write_file`). This avoids resource waste for sessions that don't use container tools.

##### Design Decision: Dev-Only Availability

This capability is experimental and only available in development environments due to:
- Security implications of host networking
- Resource management considerations
- API stability concerns

#### CodeSandbox

- **ID**: `codesandbox` (Dev only)
- **Purpose**: Cloud-based sandbox VMs via CodeSandbox API for isolated code execution
- **Dependencies**: `session_storage` (API key stored as session secret `CSB_API_KEY`)
- **Tools**: `csb_create_sandbox`, `csb_exec`, `csb_exec_status`, `csb_read_file`, `csb_write_file`, `csb_download_workspace`, `csb_list_sandboxes`, `csb_manage_sandbox`
- **Architecture**: Two-tier API — Management API for lifecycle, Pint API for in-sandbox operations

##### Design Decision: Multiple Sandboxes Per Session

Unlike DockerContainer (single container per session), CodeSandbox supports multiple sandboxes. This enables workflows like running frontend and backend in separate sandboxes, A/B testing configurations, or parallel execution.

##### Design Decision: Encrypted State Storage

Pitcher URL and token are persisted in session secrets (encrypted at rest) rather than plain-text KV store. The `pitcher_token` grants full access to the sandbox VM, so encryption is appropriate. This also avoids ~200ms+ latency per tool call from re-deriving connection info.

##### Design Decision: Dev-Only Availability

This capability is experimental and only available in development environments. The CodeSandbox API is external and requires an API key, and the integration is still being validated.

### Message Filters

Capabilities can contribute message filters that modify how messages are retrieved from the database. This enables features like:

- **Time-based filtering**: Load messages from a specific time range
- **Event type filtering**: Filter by message type (user, agent, tool result)
- **Tool name filtering**: Filter tool results by specific tool names
- **Full-text search**: Search message content
- **Ephemeral message injection**: Add system reminders or summaries without persistence

#### How Message Filters Work

1. **Filter Contribution**: Capabilities implement `message_filter_provider()` returning a `MessageFilterProvider`
2. **Priority Ordering**: Providers are sorted by priority (lower = earlier)
3. **Query Building**: Filters are combined into a `MessageQuery` and applied to message retrieval
4. **DB Mapping**: Most filters map directly to SQL WHERE clauses for efficiency
5. **In-Memory Fallback**: Custom predicates use Rust closures for complex filtering

#### MessageFilter Types

| Filter | Description | SQL Mapping |
|--------|-------------|-------------|
| `TimeRange { from, to }` | Filter by timestamp range | `WHERE created_at >= $from AND created_at <= $to` |
| `EventTypes(Vec<String>)` | Whitelist event types | `WHERE event_type = ANY($types)` |
| `ToolName(String)` | Filter tool results by name | `WHERE event_type = 'tool.completed' AND data->>'tool_name' = $name` |
| `Search(String)` | Full-text search in content | `WHERE data::text ILIKE '%' || $query || '%'` |
| `ExcludeIds(Vec<Uuid>)` | Exclude specific message IDs | `WHERE id != ALL($ids)` |
| `IncludeIds(Vec<Uuid>)` | Include only specific IDs | `WHERE id = ANY($ids)` |
| `Custom(Arc<dyn Fn(&Message) -> bool>)` | In-memory predicate | Applied after DB query |

#### MessageFilterProvider Trait

See `crates/core/src/message_filter.rs` for `MessageFilterProvider`, `MessageFilter`, `MessageQuery`, `InjectedMessage`, and `InjectionPosition` types.

#### Message Injection

Ephemeral messages can be injected into the result set without persistence (summaries, reminders, context). Injections applied after filtering at positions: `Start`, `End`, `BeforeIndex(n)`, `AfterIndex(n)`.

#### Filter Application Flow

1. **Collect Providers**: During capability resolution, filter providers are collected from enabled capabilities
2. **Sort by Priority**: Providers are sorted (lower priority values first)
3. **Build Query**: Each provider modifies the `MessageQuery` in priority order
4. **Execute Query**: The query is executed against the message store
5. **Apply Custom Filters**: In-memory predicates filter the results
6. **Apply Injections**: Ephemeral messages are inserted at their specified positions

#### Design Decisions

| Question | Decision |
|----------|----------|
| Where are filters defined? | Capabilities implement `message_filter_provider()` |
| How are they combined? | AND semantics - all filters must match |
| Order of application? | By `priority()` value (lower = earlier) |
| Can filters be stacked? | Yes - multiple capabilities can each contribute filters |
| Database efficiency? | Most filters map to SQL; only `Custom` requires in-memory filtering |
| DEV_MODE parity? | In-memory storage implements the same filter semantics |

### Extension Points (Future)

1. **Custom Capabilities**: User-defined capabilities with custom tools
2. **Conflict Resolution**: Handle tool name conflicts between capabilities
3. **Capability Versioning**: Track capability API versions for compatibility
4. **Dynamic Mount Sources**: External file sources (URLs, S3, etc.)
5. **OR Filter Semantics**: Support for OR combinations of filters
