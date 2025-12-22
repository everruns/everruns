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
│        (receives fully-configured AgentConfig)           │
└─────────────────────────────────────────────────────────┘
```

- Capabilities are defined in **everruns-core** (trait implementations)
- The API layer uses the core registry and converts to DTOs for responses
- The Agent Loop remains focused on execution
- AgentConfig is built with merged system prompt and tools from capabilities

### Data Model

#### Capability (Public DTO)

| Field | Type | Description |
|-------|------|-------------|
| `id` | CapabilityId | Unique string identifier |
| `name` | string | Display name |
| `description` | string | Description of functionality |
| `status` | CapabilityStatus | Availability status |
| `icon` | string? | Icon name for UI rendering |
| `category` | string? | Category for grouping in UI |

#### CapabilityId (String wrapper)

Capability IDs are now string-based for extensibility. New capabilities can be added without database migrations.

```rust
pub struct CapabilityId(String);

impl CapabilityId {
    // Built-in capability ID constants
    pub const NOOP: &'static str = "noop";
    pub const CURRENT_TIME: &'static str = "current_time";
    pub const RESEARCH: &'static str = "research";
    pub const SANDBOX: &'static str = "sandbox";
    pub const FILE_SYSTEM: &'static str = "file_system";
    pub const TEST_MATH: &'static str = "test_math";
    pub const TEST_WEATHER: &'static str = "test_weather";

    // Factory methods
    pub fn new(id: impl Into<String>) -> Self;
    pub fn noop() -> Self;
    pub fn current_time() -> Self;
    // ... etc
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
}
```

The `CapabilityRegistry` in core holds all registered capability implementations. The API layer converts trait objects to DTOs using `Capability::from_core()`.

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
   - Look up `InternalCapability` from registry by string ID
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
  "total": 7
}
```

#### Set Agent Capabilities

```http
PUT /v1/agents/{agent_id}/capabilities
Content-Type: application/json

{
  "capabilities": ["current_time", "test_math"]
}

Response:
{
  "items": [
    { "capability_id": "current_time", "position": 0 },
    { "capability_id": "test_math", "position": 1 }
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
    -- Capability ID is a string; validation happens at application layer
    capability_id VARCHAR(50) NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(agent_id, capability_id)
);

CREATE INDEX idx_agent_capabilities_agent_id ON agent_capabilities(agent_id);
CREATE INDEX idx_agent_capabilities_position ON agent_capabilities(agent_id, position);
```

**Note**: The `capability_id` column no longer has a CHECK constraint. Validation is performed at the application layer via `CapabilityRegistry`. This allows adding new capabilities without database migrations.

### Design Decisions

| Question | Decision |
|----------|----------|
| Where are capabilities defined? | In-memory registry (not database) |
| How are they applied? | Resolved at API layer, merged into AgentConfig |
| Order of application? | By `position` field (lower = earlier) |
| Can capabilities conflict? | Currently no conflict resolution; later capabilities add to earlier ones |
| Can users create custom capabilities? | Not in current version (built-in only) |
| How are new capabilities added? | Implement `Capability` trait and register in `CapabilityRegistry` - no database changes needed |

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

### Extension Points (Future)

1. **Custom Capabilities**: User-defined capabilities with custom tools
2. **Capability Composition**: Capabilities that depend on other capabilities
3. **Capability Configuration**: Per-agent settings for capabilities
4. **Conflict Resolution**: Handle tool name conflicts between capabilities
5. **Capability Versioning**: Track capability API versions for compatibility
