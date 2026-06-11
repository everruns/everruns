# Capabilities Specification

## Abstract

Capabilities are modular functionality units that extend Agent behavior. A Capability can contribute additions to the system prompt, provide tools, and modify execution behavior. Users can enable multiple capabilities on an Agent to compose functionality.

## Requirements

### Persisted Declarative Capabilities

Declarative capabilities are organization-scoped, database-backed capability
definitions with a stable capability reference shape `declarative:{name}`.
The persisted resource follows the standard dual-ID pattern: an internal UUID
primary key for storage and foreign keys, and a public API ID
`cap_<32-hex>` exposed as `id` in API responses and URLs. They cover capability
contributions that are safe to describe as data rather than custom code:

- System prompt additions
- Scoped remote MCP server configuration
- Text file mounts in the session filesystem
- Skill packages mounted under `/.agents/skills/{name}` and discovered by the
  built-in `skills` capability
- UI feature strings and dependency declarations
- *(Planned)* User-hook bundles (`user_hooks: [UserHookSpec]`) — see
  [user-hooks.md](user-hooks.md). The field is **not yet present** on
  `DeclarativeCapabilityDefinition`; when it lands, a non-empty
  `user_hooks` array must force the persisted record's `risk_level` to
  `high` and ride the admin-only assignment gate.

They do not define arbitrary server-side tools. Tool execution still comes from
built-in capabilities, MCP servers, uploaded skills, or client-side tools.
Before runtime assembly, the control plane hydrates the persisted definition
into the selected capability config. This lets dev mode and full worker mode
execute the same declarative capability without workers reading the database.

CRUD API:

- `GET /v1/capabilities` lists built-in, MCP, skill, and active declarative
  capability refs in one registry response.
- `POST /v1/capabilities` creates a persisted declarative capability.
- `GET /v1/capabilities/declarative` lists persisted declarative capability
  resources, including archived records when requested.
- `GET /v1/capabilities/declarative/{public_id}`
- `PATCH /v1/capabilities/declarative/{public_id}`
- `DELETE /v1/capabilities/declarative/{public_id}`
- `POST /v1/capabilities/declarative/{public_id}/delete`

The capability reference used on agents, harnesses, sessions, search results,
and MCP/catalog surfaces is `declarative:<unique_name>`. Names are unique per
organization, lowercase, bounded to fit existing capability reference columns,
and validated before persistence.

Agent and harness write APIs also accept the plain unique name as a convenience:
`{ "ref": "research_pack" }` is normalized to `{ "ref": "declarative:research_pack" }`
before validation and persistence when no built-in capability with that ID
exists. Built-in IDs keep priority to avoid ambiguous references.

Declarative capability resources store both:

- `name`: unique addressable name, used in `declarative:<name>`.
- `display_name`: optional user-facing label. When present, registry/search/UI
  use it as the title while retaining the unique name for references.

Security constraints:

- Records are org-scoped and policy-gated like other agent building blocks.
- Definitions are bounded by server-side size/count limits.
- File mounts are text-only and reject traversal paths.
- Skill names and bundled file paths are validated before persistence.
- MCP server URLs use the same SSRF-safe scoped-MCP validation as harness,
  agent, and session `mcpServers`.
- Declarative dependencies cannot point to other declarative capabilities,
  avoiding persisted dependency cycles across org data.

### Concept

A Capability is an abstraction that defines added functionality for an Agent:

1. **System Prompt Contribution**: Text prepended to the agent's system prompt
2. **Tool Provision**: Tools made available to the agent during execution
3. **Behavior Modification**: Influence on tool invocation and execution (future)

### Agent Handoff Capability

`agent_handoff` is a high-risk orchestration capability for delegating from one
configured Agent to another configured Agent through an allowlist and connection
gate. It is distinct from subagents and blueprints because the target is a
normal Agent resource, not inherited runtime config and not a code-defined
template. See [agent-handoff.md](agent-handoff.md).

Both `agent_handoff` and `a2a_agent_delegation` are gated behind the
`agent_delegation` feature flag (experimental, auto-enabled in dev, off in prod).
When the flag is off, neither capability is registered and neither appears in the
capability picker. Enable via `FEATURE_AGENT_DELEGATION=true` or use a
`DeploymentGrade::Dev` environment. See `specs/feature-flags.md` and EVE-506.

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
| Declarative | `declarative:{name}` | `declarative:research_pack` |

**Built-in IDs** use `snake_case` naming and are validated against the `CapabilityRegistry`. **MCP IDs** use the `mcp:` prefix followed by the MCP server's UUID. **Declarative IDs** use the `declarative:` prefix followed by the persisted definition's unique name. See `specs/mcp-servers.md` for MCP details.

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

Prompt contributions are paid on every model call, so capability prompts are
budgeted product surface, not documentation. Authors should keep prompt text to
the smallest stable behavior contract:

- Prefer one or two dense sentences over headings, catalogs, examples, and
  step-by-step tutorials.
- Do not include setup instructions, marketing copy, tool inventories, or
  parameter/action lists when capability metadata, tool descriptions, schemas,
  config schemas, or tool errors already carry that information.
- Do not paste external toolkit `llmtxt`/help output into the runtime prompt
  unless the model needs that grammar on every turn. Long previews may be
  exposed through `system_prompt_preview()` or docs instead.
- Config-aware prompts must describe only behavior that changes with config, and
  must not mention options that are not actually enabled in the current config.
- When a larger prompt is unavoidable (for example, UI component grammars or
  machine-readable output catalogs), document why it cannot live in schemas,
  mounted files, previews, or a tool result.

Prompt-size tests should measure the **actual assembled contribution** using
`system_prompt_contribution(ctx)` or
`system_prompt_contribution_with_config(ctx, config)`, including the
`<capability id="…">` wrapper and config-specific branches. Any capability that
adds or materially changes a runtime prompt should add or update a byte-budget
ratchet test in the same PR. Lowering a budget is always acceptable; raising a
budget requires an explicit code change and a short justification in the commit
or PR description.

##### Config-Aware Methods

- **`tools_with_config(config)`** — Returns tools adapted to per-capability config. Default delegates to `tools()`. Example: `WebFetchCapability` enables `save_to_file` when config has `enable_file_download: true`.

##### Config Schema and UI Metadata

Capabilities that accept per-agent config expose a standard JSON Schema through
`config_schema()` and optional react-jsonschema-form UI hints through
`config_ui_schema()`. These fields are copied to `CapabilityInfo` so clients
render settings through the same generic editor for built-in and integration
capabilities. The backend remains authoritative for config semantics through
`validate_config(config)`, which is called on capability write paths before
config is persisted.

##### Outbound Network Calls

Capabilities that call external HTTP/API endpoints must use `ToolContext`'s
`EgressService` and must pass `ToolContext.network_access` when the destination
URL is authored by the agent, session, capability config, or user input. New
capabilities must not construct direct external HTTP clients in tool execution.
See [egress.md](egress.md) and [network-access.md](network-access.md).

##### Collection Functions

- **`collect_capabilities()`** — Sync. Collects tools, mounts, static prompts.
- **`collect_capabilities_async(ctx)`** — Async. Calls `system_prompt_contribution()` for dynamic prompts.
- **`collect_capabilities_with_configs(ctx, configs)`** — Async. Calls config-aware methods (`tools_with_config`, `system_prompt_contribution_with_config`) for capabilities with per-agent config.
- **`RuntimeAgentBuilder::with_capabilities_async(ctx)`** — Async builder method.

**Note**: `message_filter_provider()` returns an optional filter that modifies message retrieval. See [Message Filters](#message-filters).

##### Capability-Contributed Skills

Capabilities may ship reusable skills in code via `contribute_skills() -> Vec<SkillContribution>` (default empty). Each `SkillContribution` carries a name, description, SKILL.md body, bundled files, and invocability flags. During capability collection each contribution is normalized into a read-only mount at `/.agents/skills/{name}/` containing a reconstructed `SKILL.md` plus bundled files. The built-in `skills` capability then discovers and serves them through the same VFS scan used for filesystem and registry skills — no parallel pipeline, no special-case prompt injection, and `/slash` invocability + `disable-model-invocation` flags are honored through the same frontmatter. See `specs/skills-registry.md` for the discovery/activation contract and `crates/core/src/capabilities/attach_skill.rs` for `SkillContribution` and the underlying mount construction.

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
| `gpt_image_gen` | `session_file_system`, `session_storage` | *(none)* |

#### Tool-Side Provider Clients

Some capabilities call provider APIs directly instead of routing through the LLM driver abstraction. For those cases the worker-side `ToolContext` may include:

- `provider_credential_store` for resolving default provider credentials without reading provider environment variables in tool code
- `image_store` for durable image artifact persistence and later reuse by ID

`gpt_image_gen` is the first built-in example of this pattern: it resolves OpenAI credentials through session secrets or the provider credential store, persists generated outputs in org-scoped image storage, and can optionally mirror them into the session filesystem.

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
| `session_sandbox` | `managed_sandbox` |
| `session_sql_database` | `sql_database` |

| `openui` | `openui` |
| `a2ui` | `a2ui` |

MCP and Skill virtual capabilities currently declare no features.

#### Known Feature Strings

| Feature | UI Element | Description |
|---------|------------|-------------|
| `file_system` | Workspace tab | Session has file system access |
| `secrets` | Storage tab | Session can store encrypted secrets |
| `managed_sandbox` | Sandbox-aware UX | Session has one managed sandbox with provider-neutral tools |
| `key_value` | Storage tab | Session can store key/value pairs |
| `schedules` | Schedules tab | Session can create/manage schedules |
| `sql_database` | *(reserved)* | Session has SQL database access |
| `openui` | OpenUI rendering | Session can render OpenUI Lang components |
| `a2ui` | A2UI rendering | Session can render A2UI JSON component trees |

#### compute_features()

See `crates/core/src/capabilities/mod.rs` for `compute_features()` — resolves dependencies, collects features, deduplicates.

### Risk Levels (TM-AGENT-005)

Each capability declares a `RiskLevel` via the `Capability` trait. The API enforces that assigning high-risk capabilities to an agent requires `OrgRole::Admin`.

| Level | Description | Enforcement |
|-------|-------------|-------------|
| `low` | Default. No special requirements. | Any org member |
| `medium` | Logged but allowed for org members. | Any org member |
| `high` | Can access external compute or resources, or perform unrestricted outbound network calls, or run scripted code execution. | Requires Admin role |

High-risk built-in capabilities: `docker_container`, `daytona`, `e2b`, `deno`, `virtual_bash`, `web_fetch`.

See `crates/core/src/capabilities/mod.rs` for the `RiskLevel` enum and `crates/server/src/api/agents.rs` for `require_admin_for_high_risk()`.

#### Admin-Only Tier Decision (PRs #1485, #1500, EVE-395)

`virtual_bash` (previously `Low`) and `web_fetch` (previously `Medium`) were elevated to `High` and the high-risk admin gate now applies to them.

- **Trust rationale.**
  - `virtual_bash` is sandboxed (workspace-only FS, no real network), but exposes scripted code execution. Combined with LLM-driven invocation it is a meaningful trust elevation versus single-purpose tools.
  - `web_fetch` doubles as a data-exfiltration channel (TM-AGENT-013) and a partial SSRF vector. Loopback/RFC1918 are blocked at the fetchkit layer (TM-API-008), but no general outbound URL allowlist exists yet (TM-AGENT-018 is OPEN). Admin assignment is the explicit trust gate until a per-org outbound policy lands.
- **Gate location.** Canonical agent create / update enforcement lives in `check_high_risk_caps` (`crates/server/src/domains/agents/commands.rs`), invoked from `CreateAgent::execute`, `UpdateAgent::execute`, and `UpsertAgent::execute`. The sibling `require_admin_for_high_risk` helper in `crates/server/src/api/agents.rs` enforces the same contract on agent-import / copy paths. Member-attempted assignments are rejected with HTTP 403 and an error message that names the offending capability ids.
- **Migration / grandfathering.** The gate is *not* enforced at runtime. Member-owned agents that already had `virtual_bash` or `web_fetch` before this change continue to run. Members can still edit other fields on those agents; the gate fires only when a member request would *add or keep* a high-risk capability that violates the role contract on a write that is itself reshaping the capability set.
- **Member experience.** Members trying to add `virtual_bash` or `web_fetch` see a 403 with the locked capability id. Admin approval is the path: an admin must either own the agent or perform the update on the member's behalf. A self-serve "request admin approval" UI flow is not part of this contract; it is tracked separately and may be added later without breaking the gate.
- **What this is *not*.** This is not a per-capability policy override. There is no env var or org setting today that downgrades `virtual_bash` / `web_fetch` to member-assignable. If product later wants a tunable, it will be added explicitly (org-level policy `members_can_use_high_risk_capabilities` per-capability override) rather than emerging from constant edits in capability source files.

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
- `file_store`: A `SessionFileSystem` trait implementation for file operations

See `specs/file-store.md` for the pluggable `SessionFileSystem` contract and
the in-memory and real-disk implementations. New capabilities that need
filesystem access should go through `ToolContext.file_store` or
`SystemPromptContext.file_store` rather than calling `std::fs` directly,
except for host-process tools (e.g. the bash tool) where the shell
inherits the host filesystem regardless of which `SessionFileSystem` is plugged
in.
The active session filesystem is selected by the platform's
`SessionFileSystemFactory` unless an embedder supplies an explicit runtime
backend override.

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

The `SessionFileSystemAdapter` implements bashkit's `FileSystem` trait, bridging bash file operations to the session filesystem. This enables:
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

#### A2UI

- **ID**: `a2ui`
- **Purpose**: Enables agents to emit Google A2UI JSON component trees, rendered by the UI with native shadcn/ui primitives
- **Tools**: None (prompt-only capability)
- **Features**: `a2ui`
- **Icon**: `layout-grid`
- **Category**: `UI`

##### Design Decision: Parallel to OpenUI

A2UI is a *parallel* capability to `openui`, not a replacement. Both are opt-in per agent; neither is enabled by default. A2UI uses JSON (portable across frameworks) while OpenUI uses a custom DSL with a richer prebuilt React library. Enabling both is legal but usually wasteful.

##### Design Decision: Prompt-First, Native Renderer

Follows the A2UI v0.9 prompt-first model: the catalog is embedded in the system prompt and the LLM emits JSON freely, validated lightly at render time. The UI renderer is implemented using the project's own shadcn/ui primitives — no third-party npm dependency. This preserves design-system consistency and is A2UI's canonical pattern (the client owns the rendering).

See `specs/a2ui.md` for full architecture, the canonical catalog, and the wire format.

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

#### MessageMetadata

- **ID**: `message_metadata`
- **Purpose**: Annotates user and agent messages with metadata (message timestamp, UTC) in the prompt-facing model view so the model can reason about timing and gaps between messages
- **Tools**: None (uses `ModelViewProvider` only; priority 100, after compaction masking)
- **Config**: `{"fields": ["timestamp"]}` — which metadata fields to render, in order (default `["timestamp"]`; `[]` disables annotations). User and agent messages are always annotated; system and tool-result messages never are.
- **Source**: `crates/core/src/capabilities/message_metadata.rs`
- **Behavior**: Prefixes the first text part of each user/agent message with one bracketed segment per configured field (e.g. `[time <RFC3339 UTC>]` from `Message::created_at`); tool-call-only agent messages get the annotation as a leading text part. Fields are an extensible enum (`MessageMetadataField`); future fields (e.g. the LLM model behind an agent message, once stored messages record it) render their own segment and may skip messages that lack the data. Stored messages are unchanged, and `created_at` is immutable, so annotations are deterministic across turns and do not invalidate provider prompt caches. A small system prompt addition explains the annotation and forbids the model from emitting it.

#### PromptCanaryGuardrail

- **ID**: `prompt_canary_guardrail`
- **Purpose**: Streaming output guardrail that withholds the assistant message when the model echoes the first sentence of its system prompt
- **Tools**: None (uses `output_guardrails()` only)
- **Config**: `{"replacement": "..."}` — optional replacement text shown in place of the leak (default: a generic "withheld" message)
- **Risk**: `Low` — read-only inspection of model output
- **Source**: `crates/core/src/capabilities/prompt_canary_guardrail.rs`
- **Behavior**: At stream arm time, extracts the first sentence of the assembled system prompt whose normalized form is ≥ 30 chars and uses it as a substring needle (lowercased, whitespace-collapsed). Each batched delta runs the substring check; on hit, the stream is aborted and `output.message.replaced` is emitted. The original tokens are never persisted or replayed. See [Output Guardrails](#output-guardrails).

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
     - Instruction file content: `<agent-instructions source="{path}">`
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
- `MountSource`: `InlineFile` (content + encoding), `InlineDirectory` (HashMap of entries), or `Virtual` (in-memory `VirtualFileTree` via `Arc`)
- Capabilities declare mounts via the `mounts()` trait method

#### Virtual Mounts

Virtual mounts (`MountSource::Virtual`) serve content from an in-memory `VirtualFileTree` without writing rows to the `session_files` table. Content is typically embedded at compile time via `include_dir!` and shared across sessions via `Arc`. Virtual mounts are always read-only.

- **Registration**: During `apply_capability_mounts()`, virtual mounts are registered in the `VirtualMountRegistry` (per-session, in-memory) instead of being written to the database.
- **Read path**: `SessionFileService`, `DirectWorkerAdapters`, and virtual bash all check the virtual registry before querying the database. Virtual files win on path conflicts.
- **Merge semantics**: `list_directory` merges virtual and DB entries (virtual wins on name conflict, sorted dirs-first then by path). `grep` searches both sources and concatenates results.
- **Write protection**: Writes/deletes to virtual paths return a readonly error.
- **Eviction**: Virtual mount entries for a session are evicted from the registry on session delete.
- **Lazy reconstruction**: Content is deterministic (compiled in), so the registry can be reconstructed from session capabilities on server restart.
- **Implementation**: See `crates/server/src/services/virtual_mount_registry.rs` for `VirtualMountRegistry` and `crates/core/src/capability_types.rs` for `VirtualFileTree`.

#### Mount Application Flow

1. Session is created via POST `/v1/sessions` (with `agent_id` in body)
2. SessionService fetches agent's capabilities
3. Mounts are collected from all enabled capabilities
4. Inline mounts: files and directories are created in session filesystem
5. Virtual mounts: registered in `VirtualMountRegistry` (no DB writes)
6. Files from readonly mounts are marked `is_readonly = true`

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
- **System Prompt**: Describes available management tools, common workflows, and platform docs availability
- **Mounts**: `/docs` — Everruns platform documentation (virtual, readonly). The stored/normalized mount path is `/docs`; agents/tools access it as `/workspace/docs`. Embedded at compile time via `include_dir!` from the repo `docs/` directory. Only markdown files (`.md`, `.mdx`) are included in the virtual tree.
- **Lifecycle Parity**: Must enforce the same archive/delete, assignment, and read-only rules as the public API and UI. Agents using these tools may not bypass lifecycle restrictions.
- **Tools**: Read/write split — read tools (`read_capabilities`, `read_harnesses`, `read_agents`, `read_sessions`, `session_context_report`, `session_read_messages`, `session_read_response`) return single item by ID, filtered lists, or latest session context usage; write tools (`manage_harnesses`, `manage_agents`, `manage_sessions`, `session_send_message`) perform mutations. See `crates/core/src/capabilities/platform_management.rs` for full tool parameter definitions.

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

##### Design Decision: Authorization Happens In Tool Execution, Not Harness Removal

`platform_management` remains assigned to the built-in `platform-chat` harness. Authorization must be enforced when platform tools execute, using the session owner's caller context plus the active `PermissionResolver`. Do not "fix" permission bugs by stripping the capability from Platform Chat; that removes the feature instead of fixing the broken auth boundary.

### Experimental Capabilities

Experimental capabilities are available in development environments only (`DeploymentGrade::Dev`). They may change significantly or be removed.

#### Lua execution (`lua`) and code-mode routing (`lua_code_mode`)

- `lua` — sandboxed Lua 5.4 interpreter over the session workspace. High risk,
  admin-gated, `FEATURE_LUA` + `lua` cargo feature. See
  [lua-execution.md](lua-execution.md).
- `lua_code_mode` — depends on `lua`; hides code-mode-eligible tools from the
  model's direct tool list (via a `ToolDefinitionHook`) so the agent
  orchestrates them inside a `lua` script through `tools.<name>(args)`. This is
  the canonical example of using the tool-filtering extension point to reshape
  the agent's action surface. See
  [lua-execution.md](lua-execution.md#code-mode-routing-capability-lua_code_mode).

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

#### MessageFilter Types and Provider Trait

For the full `MessageFilter` enum variants (TimeRange, EventTypes, ToolName, Search, ExcludeIds, IncludeIds, Custom) and `MessageFilterProvider` trait, see `crates/core/src/message_filter.rs`. Most filters map directly to SQL WHERE clauses; `Custom` uses in-memory predicates applied after DB query.

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

### Output Guardrails

Capabilities can contribute streaming output guardrails that inspect the model's text deltas as they arrive and, post factum, replace the assistant message when a violation is detected. This is the same extension pattern as message filters, but operates on the agent's outgoing stream rather than the message history.

#### How Output Guardrails Work

1. **Contribution**: Capabilities implement `output_guardrails()` returning a list of `Arc<dyn OutputGuardrail>` providers
2. **Arming**: At the start of each assistant message stream, `ReasonAtom` calls `OutputGuardrail::arm(ctx)` per provider. `ctx` carries the fully assembled system prompt and the contributing capability's per-agent config. Providers may return `None` to skip arming for this stream (e.g. nothing useful to derive from the prompt)
3. **Streaming check**: After each text delta is appended to the accumulated assistant text, every armed `OutputGuardrailRun::check(accumulated, delta)` runs synchronously in the streaming hot path. The contract is that `check` is cheap — substring matches, regex, hash lookups; not network calls or LLM inference
4. **Block decision**: A guardrail returns `GuardrailDecision::Block { reason_code, replacement }` to abort the stream. The pending (unsuppressed) delta is dropped, the LLM stream is dropped, and `output.message.replaced` is emitted. The replacement text becomes the assistant message body in `output.message.completed`; the original tokens are never persisted or replayed on subsequent turns
5. **Pass decision**: `GuardrailDecision::Pass` lets streaming continue. The next delta runs the check again

#### Streaming Timeline With a Trip

```
output.message.started
        │
        ▼
output.message.delta  ← (model text accumulating)
output.message.delta
        │
        ▼            (guardrail trips on the next delta — pending delta is suppressed)
output.message.replaced  ← UI discards accumulated text, shows replacement
        │
        ▼
output.message.completed  ← persisted message body = replacement
```

#### Trait + Helpers

See `crates/core/src/output_guardrail.rs` for `OutputGuardrail`, `OutputGuardrailRun`, `OutputGuardrailContext`, `GuardrailDecision`, `GuardrailBlock`, `ArmedGuardrail`, and the `evaluate_guardrails` helper used by `ReasonAtom`.

#### Design Decisions

| Question | Decision |
|----------|----------|
| Sync vs async `check`? | Sync. Guardrails run on every batched delta in the hot path; slow checks would throttle the stream. Heavy guardrails (e.g. an LLM moderator) must run elsewhere |
| Where do guardrails live? | On `Capability` trait, not `RuntimeAgent`. They are derived from `resolved_capability_configs` at execution time so `RuntimeAgent` stays serializable |
| What surface is checked? | Assistant text only (`output.message.delta`). `tool.output.delta` and `reason.thinking.delta` are out of scope for v1 — easy to extend later by adding a surface enum |
| Original tokens persisted? | No. After replacement, every downstream event (`llm.generation`, `output.message.completed`) carries the replacement text. The model itself never sees the leak on subsequent turns |
| What if multiple guardrails trip? | The first to return `Block` (in capability registration order) wins. Subsequent guardrails are not consulted on a blocked stream |
| Per-stream state? | Yes. `arm()` returns a fresh `Box<dyn OutputGuardrailRun>` per stream so guardrails can hold cursors, dedup tables, etc. without sharing across sessions |

### Extension Points (Future)

1. **Custom Capabilities**: User-defined capabilities with custom tools
2. **Conflict Resolution**: Handle tool name conflicts between capabilities
3. **Capability Versioning**: Track capability API versions for compatibility
4. **Dynamic Mount Sources**: External file sources (URLs, S3, etc.)
5. **OR Filter Semantics**: Support for OR combinations of filters
6. **Guardrail Surfaces**: Extend output guardrails to `tool.output.delta` and `reason.thinking.delta` via a surface enum on `OutputGuardrail`
