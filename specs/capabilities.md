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

See `crates/core/src/capability_dto.rs` for the full `CapabilityDto` struct definition.

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
- **`system_prompt_contribution_with_config(ctx, config)`** — Async, receives per-capability config JSON. Default delegates to `system_prompt_contribution(ctx)`. Used by `collect_capabilities_with_configs()`.

##### System Prompt Content Contract

System prompt additions must **not duplicate** information already present in tool definitions (names, descriptions, parameter schemas). Only include content that the model cannot infer from tool definitions alone:

- **Behavioral semantics**: when to use which tool, ordering constraints, cross-tool relationships
- **Non-schema constraints**: row limits, naming rules, workspace root paths, scheduling limits
- **Data layout**: filesystem paths where state is persisted (e.g., `/crm/customers.json`)
- **Execution semantics**: what happens when a schedule fires, nesting restrictions

If every piece of information in the prompt is already covered by tool definitions, return `None`. Listing tool names and descriptions in the system prompt is redundant because the model receives tool definitions as structured metadata alongside the prompt.

##### Config-Aware Methods

- **`tools_with_config(config)`** — Returns tools adapted to per-capability config. Default delegates to `tools()`. Example: `WebFetchCapability` enables `save_to_file` when config has `enable_file_download: true`.

##### Collection Functions

- **`collect_capabilities()`** — Sync. Collects tools, mounts, static prompts.
- **`collect_capabilities_async(ctx)`** — Async. Calls `system_prompt_contribution()` for dynamic prompts.
- **`collect_capabilities_with_configs(ctx, configs)`** — Async. Calls config-aware methods (`tools_with_config`, `system_prompt_contribution_with_config`) for capabilities with per-agent config.
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

| `openui` | `openui` |

MCP and Skill virtual capabilities currently declare no features.

#### Known Feature Strings

| Feature | UI Element | Description |
|---------|------------|-------------|
| `file_system` | Workspace tab | Session has file system access |
| `secrets` | Storage tab | Session can store encrypted secrets |
| `key_value` | Storage tab | Session can store key/value pairs |
| `schedules` | Schedules tab | Session can create/manage schedules |
| `sql_database` | *(reserved)* | Session has SQL database access |
| `openui` | OpenUI rendering | Session can render OpenUI Lang components |

#### compute_features()

See `crates/core/src/capabilities/mod.rs` for `compute_features()` — resolves dependencies, collects features, deduplicates.

### Risk Levels (TM-AGENT-005)

Each capability declares a `RiskLevel` via the `Capability` trait. The API enforces that assigning high-risk capabilities to an agent requires `OrgRole::Admin`.

| Level | Description | Enforcement |
|-------|-------------|-------------|
| `low` | Default. No special requirements. | Any org member |
| `medium` | Logged but allowed for org members. | Any org member |
| `high` | Can access external compute or resources. | Requires Admin role |

High-risk built-in capabilities: `docker_container`, `daytona`, `e2b`, `deno`.

See `crates/core/src/capabilities/mod.rs` for the `RiskLevel` enum and `crates/server/src/api/agents.rs` for `require_admin_for_high_risk()`.

### Built-in Capabilities

For the full list of built-in capabilities with their tools, parameters, and system prompts, see `crates/core/src/capabilities/` (each capability has its own module).

#### FileSystem

- **ID**: `session_file_system`
- **Purpose**: Tools to access and manipulate files in the session workspace (`/workspace`)
- **Tools**: `read_file`, `write_file`, `edit_file`, `list_directory`, `grep_files`, `delete_file`, `stat_file`
- **Workspace Mount**: All paths relative to `/workspace`, normalized internally for virtual_bash integration

##### Design Decision: Context-Aware Tools

FileSystem tools require session context to access the isolated virtual filesystem. Each tool implements `requires_context() -> true` and uses `execute_with_context()` instead of the standard `execute()`. The `ToolContext` provides:
- `session_id`: The session whose filesystem to access
- `file_store`: A `SessionFileStore` trait implementation for file operations

##### Design Decision: Session Isolation

Each session has its own isolated filesystem stored in PostgreSQL. Files are session-scoped and cannot be accessed across sessions. This provides security isolation and clean separation between conversations.

##### Design Decision: Auto-Create Parents

When writing a file like `/a/b/c.txt`, parent directories `/a` and `/a/b` are automatically created if they don't exist. This follows common filesystem patterns and reduces tool call overhead.

##### Design Decision: Hash-Gated Text Edits

`edit_file` is intentionally narrower than `write_file`: it only edits existing text files, requires the current `content_hash` from `read_file` or `write_file`, applies one or more exact replacements against the original file content, and commits via compare-and-set semantics so concurrent writes fail instead of clobbering newer content. This keeps the tool model-friendly while preventing stale writes, ambiguous snippet matches, and accidental binary corruption.

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

#### OpenUI

- **ID**: `openui`
- **Purpose**: Enables agents to generate rich interactive UI (charts, tables, forms, cards) using OpenUI Lang
- **Tools**: None (prompt-only capability)
- **Features**: `openui`
- **Icon**: `layout`
- **Category**: `UI`

##### Design Decision: Prompt-Only Capability

OpenUI is a pure system prompt capability — it provides no tools. It instructs the LLM to output OpenUI Lang code in ` ```openui ` fenced code blocks. The UI frontend detects and renders these blocks using `@openuidev/react-lang` and `@openuidev/react-ui`.

##### Design Decision: Component Library from Crate

The system prompt is generated from the `everruns-openui` crate which defines the full component library (50+ components across 7 groups: Content, Tables, Charts 2D, Charts 1D, Forms, Buttons, Layout). Component signatures are included in the prompt so the LLM knows valid props.

See `specs/openui.md` for full architecture and UI integration details.

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

#### LoopDetection

- **ID**: `loop_detection`
- **Purpose**: Detects repeated identical tool calls and injects a warning to break the loop
- **Tools**: None (uses `MessageFilterProvider::post_load` only)
- **Config**: `{"threshold": N}` — number of consecutive identical tool-call batches before warning (default 3)
- **Source**: `crates/core/src/capabilities/loop_detection.rs`

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
5. Update documentation at `docs/capabilities/` — add a page for the new capability and update the overview table in `docs/capabilities/index.md`

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
- **Lifecycle Parity**: Must enforce the same archive/delete, assignment, and read-only rules as the public API and UI. Agents using these tools may not bypass lifecycle restrictions.
- **Tools** (read/write split — read tools return single item by ID or filtered list):
  - `read_capabilities` - Discover available capabilities (built-in, MCP servers, skills)
    - Parameters:
      - `id`: string - Optional capability ID to get a single capability
      - `search`: string - Optional case-insensitive filter by name, description, category, or ID
    - Returns: Single capability (when `id` given) or array of capabilities with id, name, description, type (builtin/mcp_server/skill), tools, dependencies
    - Policy: Auto
  - `read_harnesses` - Read harnesses by ID or list/filter
    - Parameters:
      - `id`: string - Optional harness ID to get a single harness (returns full detail incl. system_prompt)
    - Returns: Single harness (when `id` given) or array of harness summaries with `ui_link`
    - Policy: Auto
  - `manage_harnesses` - Harness mutations: create, update, delete, copy
    - Parameters:
      - `operation`: enum (create, update, delete, copy) - The operation to perform
      - `harness_id`: string - Required for update, delete, copy
      - `name`: string - Required for create, optional for update/copy
      - `system_prompt`: string - Required for create, optional for update
      - `description`: string - Optional for create/update
      - `capabilities`: string[] - Optional for create
    - Returns: Harness data with `ui_link`
    - Policy: Auto
  - `read_agents` - Read agents by ID or list/filter
    - Parameters:
      - `id`: string - Optional agent ID to get a single agent (returns full detail incl. system_prompt)
    - Returns: Single agent (when `id` given) or array of agent summaries with `ui_link`
    - Policy: Auto
  - `manage_agents` - Agent mutations: create, update, delete
    - Parameters:
      - `operation`: enum (create, update, delete) - The operation to perform
      - `agent_id`: string - Required for update, delete
      - `name`: string - Required for create, optional for update
      - `system_prompt`: string - Required for create, optional for update
      - `description`: string - Optional for create/update
      - `capabilities`: string[] - Optional for create
    - Returns: Agent data with `ui_link`
    - Policy: Auto
  - `read_sessions` - Read sessions by ID or list/filter
    - Parameters:
      - `id`: string - Optional session ID to get a single session
      - `agent_id`: string - Optional filter by agent
      - `limit`: integer - Optional max results for list (default: 20)
    - Returns: Single session (when `id` given) or array of session summaries with `ui_link`
    - Policy: Auto
  - `manage_sessions` - Session mutations: create, delete
    - Parameters:
      - `operation`: enum (create, delete) - The operation to perform
      - `session_id`: string - Required for delete
      - `harness_id`: string - Optional for create (defaults to built-in Generic harness when omitted)
      - `agent_id`: string - Optional for create
      - `title`: string - Optional for create
    - Returns: Session data with `ui_link`
    - Policy: Auto
  - `session_send_message` - Send a user message to a session, triggering a turn
    - Parameters:
      - `session_id`: string - Target session ID
      - `content`: string - Message content
    - Returns: Confirmation with `ui_link`
    - Policy: Auto
  - `session_read_messages` - Read messages from a session
    - Parameters:
      - `session_id`: string - Target session ID
      - `limit`: integer - Optional max messages (default: 10)
    - Returns: Array of messages with role, content, created_at
    - Policy: Auto
  - `session_read_response` - Wait for session to finish processing and return the response
    - Parameters:
      - `session_id`: string - Target session ID
      - `timeout_secs`: integer - Optional timeout (default: 120). Set to 0 to check status without waiting.
    - Returns: Session status (session_id, status, ui_link)
    - Policy: Auto
- **Icon**: "settings"
- **Category**: "Management"

Lifecycle rules for platform management:
- `delete` archives.
- Archived entities can be read but not updated or assigned.
- Deleted entities should behave as missing except where historical references are intentionally rendered as tombstones.
- Future: `destroy` (hard delete) will require a dedicated `PlatformStore` method and dangerous permission gate.

##### Design Decision: Context-Aware Tools

All platform management tools require session context to access the `PlatformStore`. Each tool implements `requires_context() -> true` and uses `execute_with_context()`. The `ToolContext` provides a `platform_store` field with the `PlatformStore` implementation.

##### Design Decision: UI Links

All tool results include `ui_link` fields pointing to the relevant UI page (e.g., `/harnesses/{id}`, `/agents/{id}`, `/sessions/{id}/chat`). This lets agents direct users to the web interface for visual management.

##### Design Decision: Read/Write Tool Split

Platform management tools are split into read tools (`read_*`) and write tools (`manage_*`). Read tools accept an optional `id` parameter: when provided they return a single detailed item, otherwise they return a filtered list. This separation lets LLMs use read tools freely (no side effects) while write tools are clearly mutation-oriented. Session interaction is split into three single-purpose tools (`session_send_message`, `session_read_messages`, `session_read_response`) eliminating the operation-dispatch pattern for session I/O.

##### Design Decision: Capability Discovery via read_capabilities

The `read_capabilities` tool enables agents (particularly the Platform Chat) to discover available capabilities before creating or updating agents/harnesses. It queries the `PlatformStore.list_capabilities()` method which returns built-in capabilities, MCP server capabilities, and skill capabilities. The search parameter supports case-insensitive filtering across name, description, category, and ID. The Platform Chat harness system prompt instructs the agent to use `read_capabilities` before creating agents.

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
| `Search(String)` | Full-text search in content | `WHERE search_vector @@ plainto_tsquery('english', $query)` |
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
