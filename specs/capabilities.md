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

Capabilities are designed as an **external concern** to the Agent Loop:

```
┌─────────────────────────────────────────────────────────┐
│                     API / Service Layer                  │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────┐  │
│  │   Agent     │  │ Capabilities│  │CapabilityService│  │
│  │   Config    │←─│   Registry  │←─│  (resolution)   │  │
│  └─────────────┘  └─────────────┘  └─────────────────┘  │
└───────────────────────────┬─────────────────────────────┘
                            ↓
┌─────────────────────────────────────────────────────────┐
│                      Agent Loop                          │
│        (receives fully-configured AgentConfig)           │
└─────────────────────────────────────────────────────────┘
```

- Capabilities are resolved at the **service/API layer**
- The Agent Loop remains focused on execution
- AgentConfig is built with merged system prompt and tools from capabilities

### Data Model

#### Capability (Public DTO)

| Field | Type | Description |
|-------|------|-------------|
| `id` | CapabilityId | Unique identifier (enum) |
| `name` | string | Display name |
| `description` | string | Description of functionality |
| `status` | CapabilityStatus | Availability status |
| `icon` | string? | Icon name for UI rendering |
| `category` | string? | Category for grouping in UI |

#### CapabilityId (Enum)

```rust
pub enum CapabilityId {
    Noop,       // "noop"
    CurrentTime, // "current_time"
    Research,   // "research"
    Sandbox,    // "sandbox"
    FileSystem, // "file_system"
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

#### InternalCapability (Server-side)

Full capability definition with implementation details:

```rust
pub struct InternalCapability {
    pub info: Capability,                    // Public info
    pub system_prompt_addition: Option<String>, // Prepended to agent's system prompt
    pub tools: Vec<ToolDefinition>,          // Tools provided by this capability
}
```

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

#### FileSystem (Coming Soon)

- **Status**: ComingSoon
- **Purpose**: File system access tools
- **System Prompt**: "You have access to file system tools. You can read, write, and search files."
- **Tools**: To be added (read, write, grep, glob)
- **Icon**: "folder"
- **Category**: "File Operations"

### Capability Application Flow

When a session executes:

1. **Load Agent**: Fetch agent configuration from database
2. **Fetch Capabilities**: Get agent's enabled capabilities via `get_agent_capabilities(agent_id)`
3. **Resolve Capabilities**: For each capability (ordered by `position`):
   - Look up `InternalCapability` from registry
   - Collect `system_prompt_addition` texts
   - Collect `tools` definitions
4. **Build AgentConfig**:
   - System prompt = capability additions + agent's base system prompt
   - Tools = merged tool list from all capabilities
5. **Execute**: Run Agent Loop with fully configured AgentConfig

### API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/v1/capabilities` | List all available capabilities |
| GET | `/v1/capabilities/{capability_id}` | Get capability details |
| GET | `/v1/agents/{agent_id}/capabilities` | Get capabilities for an agent |
| PUT | `/v1/agents/{agent_id}/capabilities` | Set capabilities for an agent |

#### List Capabilities

```http
GET /v1/capabilities

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
      "id": "research",
      "name": "Research",
      "description": "Deep research capability with organized scratchpad.",
      "status": "coming_soon",
      "icon": "search",
      "category": "AI"
    }
  ],
  "total": 5
}
```

#### Set Agent Capabilities

```http
PUT /v1/agents/{agent_id}/capabilities
Content-Type: application/json

{
  "capabilities": ["current_time", "research"]
}

Response:
{
  "items": [
    { "capability_id": "current_time", "position": 0 },
    { "capability_id": "research", "position": 1 }
  ],
  "total": 2
}
```

The array order determines `position` (index becomes position value).

### Database Schema

```sql
CREATE TABLE agent_capabilities (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    agent_id UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    capability_id VARCHAR(50) NOT NULL CHECK (
        capability_id IN ('noop', 'current_time', 'research', 'sandbox', 'file_system')
    ),
    position INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(agent_id, capability_id)
);

CREATE INDEX idx_agent_capabilities_agent_id ON agent_capabilities(agent_id);
CREATE INDEX idx_agent_capabilities_position ON agent_capabilities(agent_id, position);
```

### Design Decisions

| Question | Decision |
|----------|----------|
| Where are capabilities defined? | In-memory registry (not database) |
| How are they applied? | Resolved at API layer, merged into AgentConfig |
| Order of application? | By `position` field (lower = earlier) |
| Can capabilities conflict? | Currently no conflict resolution; later capabilities add to earlier ones |
| Can users create custom capabilities? | Not in current version (built-in only) |

### Extension Points (Future)

1. **Custom Capabilities**: User-defined capabilities with custom tools
2. **Capability Composition**: Capabilities that depend on other capabilities
3. **Capability Configuration**: Per-agent settings for capabilities
4. **Conflict Resolution**: Handle tool name conflicts between capabilities
5. **Capability Versioning**: Track capability API versions for compatibility
