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

Capability IDs are string-based for extensibility. New capabilities can be added without database migrations.

##### ID Format

| Type | Format | Example |
|------|--------|---------|
| Built-in | `snake_case` identifier | `current_time`, `web_fetch` |
| MCP | `mcp:{server_uuid}` | `mcp:01933b5a-0000-7000-8000-000000000501` |

**Built-in IDs** use `snake_case` naming and are validated against the `CapabilityRegistry`. Each capability defines its own ID via the `Capability` trait's `id()` method.

**MCP IDs** use the `mcp:` prefix followed by the MCP server's UUID. These are dynamic—they exist only when the corresponding MCP server exists. See `specs/mcp-servers.md` for details.

##### Quick Reference: Built-in Capabilities

| ID | Name | Category | Status |
|----|------|----------|--------|
| `noop` | No-Op | Testing | Available |
| `current_time` | Current Time | Utilities | Available |
| `test_math` | Test Math | Testing | Available |
| `test_weather` | Test Weather | Testing | Available |
| `session_file_system` | File System | File Operations | Available |
| `session_storage` | Session Storage | Storage | Available |
| `web_fetch` | Web Fetch | Network | Available |
| `stateless_todo_list` | Task Management | Productivity | Available |
| `sample_data` | Sample Data | Data | Available |
| `docker_container` | Docker Container | Development | Available (Dev only) |
| `research` | Research | AI | Coming Soon |
| `sandbox` | Sandbox | Execution | Coming Soon |

```rust
/// Simple string wrapper - capabilities define their own IDs via id() method
pub struct CapabilityId(String);

impl CapabilityId {
    /// Create a new capability ID from a string
    pub fn new(id: impl Into<String>) -> Self;

    /// Get the ID as a string slice
    pub fn as_str(&self) -> &str;
}
```

#### CapabilityStatus (Enum)

```rust
pub enum CapabilityStatus {
    Available,   // Ready for use
    ComingSoon,  // Not yet implemented
    Deprecated,  // No longer recommended
}
```

#### AgentCapabilityConfig (Per-agent configuration)

Associates a capability with an agent, including optional per-agent configuration.

| Field | Type | Description |
|-------|------|-------------|
| `ref` | CapabilityId | Reference to the capability |
| `config` | object | Per-agent configuration (capability-specific) |

```rust
/// Per-agent capability configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCapabilityConfig {
    /// Reference to the capability ID
    #[serde(rename = "ref")]
    pub capability_ref: CapabilityId,
    /// Per-agent configuration for this capability
    #[serde(default)]
    pub config: serde_json::Value,
}
```

The `config` field is capability-specific and passed to the capability during execution.
This allows the same capability to behave differently per-agent.

**Example usage:**
```json
{
  "capabilities": [
    { "ref": "current_time", "config": {} },
    { "ref": "web_fetch", "config": { "timeout_ms": 30000 } }
  ]
}
```

**Import compatibility:** For backward compatibility, import accepts both formats:
- New format: `{ "ref": "capability_id", "config": {} }`
- Legacy format: `"capability_id"` (converted to new format with empty config)

#### Capability Trait (everruns-core)

Capabilities are defined as trait implementations in the core crate:

```rust
pub trait Capability: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn status(&self) -> CapabilityStatus;
    fn system_prompt_addition(&self) -> Option<&str> { None }
    fn tools(&self) -> Vec<Box<dyn Tool>> { vec![] }
    fn icon(&self) -> Option<&str> { None }
    fn category(&self) -> Option<&str> { None }
    fn mounts(&self) -> Vec<MountPoint> { vec![] }
    fn dependencies(&self) -> Vec<&'static str> { vec![] }
    fn message_filter_provider(&self) -> Option<Arc<dyn MessageFilterProvider>> { None }
}
```

The `CapabilityRegistry` in core holds all registered capability implementations. The API layer converts trait objects to DTOs using `Capability::from_core()`.

**Note**: The `message_filter_provider()` method returns an optional filter provider that can modify how messages are retrieved for sessions using this capability. See the [Message Filters](#message-filters) section for details.

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

```rust
impl Capability for SampleDataCapability {
    fn id(&self) -> &str { "sample_data" }

    fn dependencies(&self) -> Vec<&'static str> {
        // Sample Data needs file system tools to be useful
        vec!["session_file_system"]
    }
}
```

When `sample_data` is selected:
1. `session_file_system` is added as a dependency (if not already selected)
2. `session_file_system` tools and system prompt are applied first
3. `sample_data` mounts and system prompt are applied second
4. User cannot remove `session_file_system` while `sample_data` is selected

#### Dependency Resolution API

```rust
/// Result of resolving capability dependencies
pub struct ResolvedCapabilities {
    /// All capability IDs after resolving dependencies (in topological order)
    pub resolved_ids: Vec<String>,
    /// IDs that were added as dependencies (not in the original selection)
    pub added_as_dependencies: Vec<String>,
    /// Original user-selected capability IDs
    pub user_selected: Vec<String>,
}

/// Resolve capability dependencies
pub fn resolve_dependencies(
    selected_ids: &[String],
    registry: &CapabilityRegistry,
) -> Result<ResolvedCapabilities, DependencyError>;
```

#### Built-in Capabilities with Dependencies

| Capability | Depends On |
|------------|-----------|
| `sample_data` | `session_file_system` |

### Built-in Capabilities

#### Noop

- **Status**: Available
- **Purpose**: Testing and demonstration
- **System Prompt**: None
- **Tools**: None

#### CurrentTime

- **Status**: Available
- **Purpose**: Get current date and time in various formats
- **System Prompt**: None
- **Tools**:
  - `get_current_time` - Returns current timestamp
    - Parameters:
      - `timezone`: string (e.g., "UTC", "America/New_York")
      - `format`: enum (iso8601, unix, human)
    - Policy: Auto

#### TestMath

- **Status**: Available
- **Purpose**: Testing tool calling with calculator operations
- **System Prompt**: "You have access to math tools. Use them for calculations: add, subtract, multiply, divide."
- **Tools**:
  - `add` - Add two numbers
  - `subtract` - Subtract second number from first
  - `multiply` - Multiply two numbers
  - `divide` - Divide first number by second
- **Icon**: "calculator"
- **Category**: "Testing"

#### TestWeather

- **Status**: Available
- **Purpose**: Testing tool calling with mock weather data
- **System Prompt**: "You have access to weather tools. Use get_weather for current conditions and get_forecast for multi-day forecasts."
- **Tools**:
  - `get_weather` - Get current weather for a city
  - `get_forecast` - Get multi-day forecast
- **Icon**: "cloud-sun"
- **Category**: "Testing"

#### Research (Coming Soon)

- **Status**: ComingSoon
- **Purpose**: Deep research with organized findings
- **System Prompt**: "You have access to a research scratchpad. Use it to organize your thoughts and findings."
- **Tools**: To be added (scratchpad, web search, etc.)
- **Icon**: "search"
- **Category**: "AI"

#### Sandbox (Coming Soon)

- **Status**: ComingSoon
- **Purpose**: Sandboxed code execution environment
- **System Prompt**: "You can execute code in a sandboxed environment. Use the execute_code tool to run code safely."
- **Tools**: To be added (execute_code with language support)
- **Icon**: "box"
- **Category**: "Execution"

#### FileSystem

- **Status**: Available
- **ID**: `session_file_system`
- **Purpose**: Tools to access and manipulate files in the session file system
- **System Prompt**: Guidance on using file system tools, best practices for exploration and file operations
- **Tools**:
  - `read_file` - Read file content by path
    - Parameters:
      - `path`: string (required) - Absolute path to the file (e.g., '/docs/readme.txt')
    - Returns: Object containing path, content, encoding, size_bytes
    - Policy: Auto
  - `write_file` - Create or update a file
    - Parameters:
      - `path`: string (required) - Absolute path for the file
      - `content`: string (required) - Content to write
      - `encoding`: enum (text, base64) - Content encoding, defaults to text
    - Returns: Object containing path, size_bytes, created status
    - Policy: Auto
  - `list_directory` - List files and directories at a path
    - Parameters:
      - `path`: string - Directory path to list, defaults to root '/'
    - Returns: Object containing path, entries array, count
    - Policy: Auto
  - `grep_files` - Search file contents using regex
    - Parameters:
      - `pattern`: string (required) - Regex pattern to search for
      - `path_pattern`: string - Optional path pattern filter (e.g., '*.txt')
    - Returns: Object containing pattern, matches array, match_count
    - Policy: Auto
  - `delete_file` - Delete a file or directory
    - Parameters:
      - `path`: string (required) - Path to file or directory
      - `recursive`: boolean - If true, delete directories recursively, defaults to false
    - Returns: Object containing path, deleted status
    - Policy: Auto
  - `stat_file` - Get file or directory metadata
    - Parameters:
      - `path`: string (required) - Path to the file or directory
    - Returns: Object containing path, name, exists, is_directory, is_readonly, size_bytes, timestamps
    - Policy: Auto
- **Icon**: "folder"
- **Category**: "File Operations"

##### Design Decision: Context-Aware Tools

FileSystem tools require session context to access the isolated virtual filesystem. Each tool implements `requires_context() -> true` and uses `execute_with_context()` instead of the standard `execute()`. The `ToolContext` provides:
- `session_id`: The session whose filesystem to access
- `file_store`: A `SessionFileStore` trait implementation for file operations

##### Design Decision: Session Isolation

Each session has its own isolated filesystem stored in PostgreSQL. Files are session-scoped and cannot be accessed across sessions. This provides security isolation and clean separation between conversations.

##### Design Decision: Auto-Create Parents

When writing a file like `/a/b/c.txt`, parent directories `/a` and `/a/b` are automatically created if they don't exist. This follows common filesystem patterns and reduces tool call overhead.

#### SessionStorage

- **Status**: Available
- **ID**: `session_storage`
- **Purpose**: Session-scoped key/value storage and encrypted secret storage
- **System Prompt**: Guidance on using storage tools for persisting data and secrets
- **Tools**:
  - `kv_store` - Key/value storage operations (plain text)
    - Parameters:
      - `operation`: enum (set, get, delete, list) - The operation to perform
      - `key`: string (max 255 chars) - Required for set, get, delete
      - `value`: string - Required for set (can be JSON-encoded)
    - Returns: Object containing operation, key, and result (success/value/deleted/keys)
    - Policy: Auto
  - `secret_store` - Encrypted secret storage operations
    - Parameters:
      - `operation`: enum (set, get, delete, list) - The operation to perform
      - `name`: string (max 255 chars) - Required for set, get, delete
      - `value`: string - Required for set (will be encrypted)
    - Returns: Object containing operation, name, and result (success/value/deleted/secrets)
    - Policy: Auto
- **Icon**: "database"
- **Category**: "Storage"

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

- **Status**: Available
- **ID**: `web_fetch`
- **Purpose**: Fetch content from URLs and convert HTML to markdown or plain text
- **System Prompt**: None (this capability does not add to the system prompt)
- **Timeouts**:
  - **First byte timeout**: 1 second - If server doesn't respond within 1 second, request fails
  - **Body timeout**: 30 seconds - If body isn't fully read within 30 seconds, partial content is returned with `[..more content timed out...]` suffix
- **Tools**:
  - `web_fetch` - Fetch content from a URL
    - Parameters:
      - `url`: string (required) - The URL to fetch, must start with http:// or https://
      - `method`: enum (GET, HEAD) - HTTP method, defaults to GET
      - `as_markdown`: boolean - Convert HTML response to markdown format
      - `as_text`: boolean - Convert HTML response to plain text (ignored if as_markdown is true)
    - Returns: Object containing:
      - `url`: The requested URL
      - `status_code`: HTTP status code
      - `content_type`: Response content type
      - `size`: Number of bytes received
      - `last_modified`: Last-Modified header value (when available)
      - `filename`: Extracted filename from Content-Disposition header or URL (when available)
      - `format`: "markdown", "text", or "raw" depending on conversion
      - `content`: The fetched content (not present for HEAD requests or binary content)
      - `truncated`: boolean - True if body was truncated due to timeout
      - `error`: (for binary content) Error message explaining binary content is not supported
    - Error handling:
      - Binary content (images, PDFs, etc.) returns success with metadata and error message (not a tool error)
      - Invalid URLs return validation errors
      - Network errors return appropriate error messages
      - First byte timeout returns "server did not respond within 1 second"
    - Policy: Auto
- **Icon**: "globe"
- **Category**: "Network"

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

The capability uses the [fetchkit](https://github.com/everruns/fetchkit) library for all HTTP operations and HTML conversion. This provides:
- Consistent behavior across deployments
- Built-in HTML to markdown/text conversion optimized for LLM consumption
- Automatic binary content detection
- Timeout management with partial content on body timeout
- Excessive newline filtering

#### StatelessTodoList

- **Status**: Available
- **ID**: `stateless_todo_list`
- **Purpose**: Enable agents to create and manage structured task lists for tracking multi-step work progress
- **System Prompt**: Comprehensive guidance on when and how to use task management, including best practices for multi-step workflows
- **Tools**:
  - `write_todos` - Create or update a task list
    - Parameters:
      - `todos`: array of task objects, each with:
        - `content`: string (imperative form, e.g., "Run tests", "Fix the bug")
        - `activeForm`: string (present continuous, e.g., "Running tests", "Fixing the bug")
        - `status`: enum (pending, in_progress, completed)
    - Returns: success status with task counts and validated todos
    - Validation: Warns if no task is in_progress (when pending tasks exist) or if multiple tasks are in_progress
    - Policy: Auto
- **Icon**: "list-checks"
- **Category**: "Productivity"

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
2. **Fetch Capabilities**: Get agent's enabled capabilities via `get_agent_capabilities(agent_id)`
3. **Resolve Capabilities**: For each capability (ordered by `position`):
   - Look up `InternalCapability` from registry by string ID
   - Collect `system_prompt_addition` texts
   - Collect `tools` definitions
4. **Build RuntimeAgent**:
   - System prompt = capability additions + agent's base system prompt
   - Tools = merged tool list from all capabilities
5. **Execute**: Run Agent Loop with fully configured RuntimeAgent

### API Endpoints

Capabilities are managed as part of the agent resource. When creating or updating an agent, you can specify the capabilities to enable. The agent response includes the list of enabled capabilities.

All endpoints are organization-scoped.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/orgs/{org}/capabilities` | List all available capabilities |
| GET | `/v1/orgs/{org}/capabilities/{capability_id}` | Get capability details |

Agent capabilities are managed through the agents API:
- `POST /v1/orgs/{org}/agents` - Create agent with capabilities
- `PATCH /v1/orgs/{org}/agents/{id}` - Update agent capabilities
- `GET /v1/orgs/{org}/agents/{id}` - Get agent (includes capabilities)

#### List Capabilities

```http
GET /v1/orgs/{org}/capabilities

Response:
{
  "items": [
    {
      "id": "current_time",
      "name": "Current Time",
      "description": "Tool to get current date and time in various formats and timezones.",
      "status": "available",
      "icon": "clock",
      "category": "Utilities"
    },
    {
      "id": "web_fetch",
      "name": "Web Fetch",
      "description": "Fetch content from URLs and convert HTML responses to markdown or plain text.",
      "status": "available",
      "icon": "globe",
      "category": "Network"
    },
    {
      "id": "stateless_todo_list",
      "name": "Task Management",
      "description": "Enables agents to create and manage structured task lists for tracking multi-step work progress. State is maintained in conversation history.",
      "status": "available",
      "icon": "list-checks",
      "category": "Productivity"
    },
    {
      "id": "research",
      "name": "Research",
      "description": "Deep research capability with organized scratchpad.",
      "status": "coming_soon",
      "icon": "search",
      "category": "AI"
    }
  ],
  "total": 9
}
```

#### Create Agent with Capabilities

```http
POST /v1/orgs/{org}/agents
Content-Type: application/json

{
  "name": "Research Assistant",
  "system_prompt": "You are a helpful research assistant.",
  "capabilities": [
    { "ref": "current_time", "config": {} },
    { "ref": "test_math", "config": {} }
  ]
}

Response:
{
  "id": "...",
  "name": "Research Assistant",
  "system_prompt": "You are a helpful research assistant.",
  "capabilities": [
    { "ref": "current_time", "config": {} },
    { "ref": "test_math", "config": {} }
  ],
  "status": "active",
  ...
}
```

#### Update Agent Capabilities

```http
PATCH /v1/orgs/{org}/agents/{agent_id}
Content-Type: application/json

{
  "capabilities": [
    { "ref": "current_time", "config": {} },
    { "ref": "web_fetch", "config": { "timeout_ms": 30000 } },
    { "ref": "session_file_system", "config": {} }
  ]
}

Response:
{
  "id": "...",
  "name": "Research Assistant",
  "capabilities": [
    { "ref": "current_time", "config": {} },
    { "ref": "web_fetch", "config": { "timeout_ms": 30000 } },
    { "ref": "session_file_system", "config": {} }
  ],
  ...
}
```

The array order determines the priority (earlier capabilities' system prompt additions appear first).

### Database Schema

```sql
CREATE TABLE agent_capabilities (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    -- Capability ID is a string; validation happens at application layer
    capability_id VARCHAR(50) NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    -- Per-agent capability configuration (JSON)
    config JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(agent_id, capability_id)
);

CREATE INDEX idx_agent_capabilities_agent_id ON agent_capabilities(agent_id);
CREATE INDEX idx_agent_capabilities_position ON agent_capabilities(agent_id, position);
```

**Note**: The `capability_id` column no longer has a CHECK constraint. Validation is performed at the application layer via `CapabilityRegistry`. This allows adding new capabilities without database migrations. The `config` column stores per-agent configuration as JSON, allowing capability-specific settings per agent.

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

To add a new capability:

1. **Implement the Capability trait**:
   ```rust
   pub struct MyNewCapability;

   impl Capability for MyNewCapability {
       fn id(&self) -> &str { "my_new_capability" }
       fn name(&self) -> &str { "My New Capability" }
       fn description(&self) -> &str { "Description here" }
       fn status(&self) -> CapabilityStatus { CapabilityStatus::Available }
       fn tools(&self) -> Vec<Box<dyn Tool>> { vec![] }
   }
   ```

2. **Register in CapabilityRegistry**:
   ```rust
   impl CapabilityRegistry {
       pub fn with_builtins() -> Self {
           let mut registry = Self::new();
           // ... existing capabilities
           registry.register(MyNewCapability);
           registry
       }
   }
   ```

3. **Add tool implementations** if needed (implement `Tool` trait)

4. **No database migration required** - the capability ID is validated at runtime

### Capability Mount Points

Capabilities can declare mount points to populate files and directories in the session filesystem when a session is created. This allows capabilities to provide sample data, documentation, configuration files, or other resources that agents can access during execution.

#### Mount Point Data Model

| Field | Type | Description |
|-------|------|-------------|
| `path` | string | Target path in session filesystem (e.g., "/samples") |
| `access` | MountAccess | Read/write permissions: `ReadOnly` or `ReadWrite` |
| `source` | MountSource | Content to mount (file or directory) |
| `capability_id` | string | ID of the capability providing this mount |

#### MountAccess

```rust
pub enum MountAccess {
    ReadOnly,   // Files cannot be modified or deleted
    ReadWrite,  // Files can be read and written
}
```

#### MountSource

```rust
pub enum MountSource {
    InlineFile {
        content: String,   // File content
        encoding: String,  // "text" or "base64"
    },
    InlineDirectory {
        entries: HashMap<String, MountEntry>,
    },
}
```

#### Capability Trait Addition

```rust
pub trait Capability: Send + Sync {
    // ... existing methods ...

    /// Returns mount points to populate in the session filesystem.
    /// Default: empty vector (no mounts).
    fn mounts(&self) -> Vec<MountPoint> { vec![] }
}
```

#### Mount Application Flow

1. Session is created via POST `/v1/agents/{agent_id}/sessions`
2. SessionService fetches agent's capabilities
3. Mounts are collected from all enabled capabilities
4. Files and directories are created in session filesystem
5. Files from readonly mounts are marked `is_readonly = true`

#### Example Capability with Mounts

```rust
impl Capability for SampleDataCapability {
    fn id(&self) -> &str { "sample_data" }
    fn name(&self) -> &str { "Sample Data" }

    fn mounts(&self) -> Vec<MountPoint> {
        let samples_dir = MountDirectoryBuilder::new()
            .file("users.json", USERS_JSON)
            .file("config.yaml", CONFIG_YAML)
            .build();

        vec![MountPoint::readonly("/samples", samples_dir, self.id())]
    }
}
```

#### Built-in Capabilities with Mounts

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

### Experimental Capabilities

Experimental capabilities are available in development environments only (`DeploymentGrade::Dev`). They may change significantly or be removed.

#### DockerContainer

- **Status**: Available (Dev only)
- **ID**: `docker_container`
- **Purpose**: Run commands and manage files in a Docker container tied to the session
- **System Prompt**: Guidance on using container tools, best practices for command execution
- **Container Lifecycle**:
  - Lazily started on first tool use
  - Persists for session duration
  - Uses host networking
  - Container name: `everruns-{session_id}`
- **Configuration** (via `AgentCapabilityConfig.config`):
  - `image`: Docker image to use (default: `mcr.microsoft.com/devcontainers/python:3.11`)
  - `working_dir`: Working directory inside container (default: `/workspace`)
- **Tools**:
  - `docker_exec` - Execute shell command in container
    - Parameters:
      - `command`: string (required) - Command to execute
      - `working_dir`: string - Override working directory
      - `config`: object - Container configuration
    - Returns: stdout, stderr, exit_code, success
    - Policy: Auto
  - `docker_read_file` - Read file from container filesystem
    - Parameters:
      - `path`: string (required) - Absolute path to file
      - `config`: object - Container configuration
    - Returns: path, content, size_bytes
    - Policy: Auto
  - `docker_write_file` - Write file to container filesystem
    - Parameters:
      - `path`: string (required) - Absolute path for file
      - `content`: string (required) - Content to write
      - `config`: object - Container configuration
    - Returns: path, size_bytes, success
    - Policy: Auto
  - `docker_logs` - Get logs from container
    - Parameters:
      - `tail`: integer - Number of lines from end (default: 100)
      - `since`: string - Timestamp or relative time (e.g., '10m', '1h')
      - `timestamps`: boolean - Include timestamps (default: false)
    - Returns: logs, stdout, stderr, container_name, lines_requested
    - Policy: Auto
  - `docker_stop` - Stop and remove container
    - Parameters:
      - `force`: boolean - Force kill (default: false)
    - Returns: stopped, removed, container_name, message
    - Policy: Auto
- **Icon**: "container"
- **Category**: "Development"

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
| `ToolName(String)` | Filter tool results by name | `WHERE event_type = 'tool.call_completed' AND data->>'tool_name' = $name` |
| `Search(String)` | Full-text search in content | `WHERE data::text ILIKE '%' || $query || '%'` |
| `ExcludeIds(Vec<Uuid>)` | Exclude specific message IDs | `WHERE id != ALL($ids)` |
| `IncludeIds(Vec<Uuid>)` | Include only specific IDs | `WHERE id = ANY($ids)` |
| `Custom(Arc<dyn Fn(&Message) -> bool>)` | In-memory predicate | Applied after DB query |

#### MessageFilterProvider Trait

```rust
pub trait MessageFilterProvider: Send + Sync {
    /// Modify the message query by adding filters and/or injections.
    fn apply_filters(&self, query: &mut MessageQuery, config: &serde_json::Value);

    /// Priority for filter application (lower = earlier). Default is 0.
    fn priority(&self) -> i32 { 0 }
}
```

#### Message Injection

Ephemeral messages can be injected into the result set without persistence:

```rust
pub enum InjectionPosition {
    Start,           // Before all messages
    End,             // After all messages
    BeforeIndex(usize),
    AfterIndex(usize),
}

pub struct InjectedMessage {
    pub position: InjectionPosition,
    pub message: Message,
}
```

Injections are applied after filtering and are useful for:
- Adding conversation summaries at the start
- Inserting system reminders before recent context
- Appending instructions or context

#### MessageQuery Builder

```rust
let query = MessageQuery::new(session_id)
    .with_filter(MessageFilter::TimeRange {
        from: Some(Utc::now() - Duration::hours(24)),
        to: None,
    })
    .with_filter(MessageFilter::EventTypes(vec!["message.user".to_string()]))
    .with_injection(InjectedMessage::at_start(Message::system("Summary: ...")))
    .with_limit(100);
```

#### Example Capability with Message Filter

```rust
impl Capability for RecentMessagesCapability {
    fn id(&self) -> &str { "recent_messages" }
    fn name(&self) -> &str { "Recent Messages" }
    fn description(&self) -> &str { "Only load messages from the last 24 hours" }

    fn message_filter_provider(&self) -> Option<Arc<dyn MessageFilterProvider>> {
        Some(Arc::new(RecentMessagesProvider))
    }
}

struct RecentMessagesProvider;

impl MessageFilterProvider for RecentMessagesProvider {
    fn apply_filters(&self, query: &mut MessageQuery, config: &serde_json::Value) {
        // Check config for custom hours setting
        let hours = config.get("hours")
            .and_then(|v| v.as_u64())
            .unwrap_or(24);

        let cutoff = Utc::now() - Duration::hours(hours as i64);
        query.filters.push(MessageFilter::TimeRange {
            from: Some(cutoff),
            to: None,
        });
    }

    fn priority(&self) -> i32 {
        -10  // Run early to reduce data loaded
    }
}
```

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
