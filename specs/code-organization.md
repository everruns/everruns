# Code Organization

## Abstract

Conventions for code organization, naming, type flow, testing, and error handling.

## Layer Separation

See `specs/architecture.md` for full layer diagrams. Summary:

```
Transport (HTTP/gRPC) → Services (Business Logic) → Storage (Database)
```

**Rules:**
- Transport handlers delegate ALL logic to services
- Services own validation, conversion, orchestration
- Storage does raw database operations only
- No direct DB access from transport layer

## Naming Conventions

| Layer | Pattern | Example |
|-------|---------|---------|
| Storage Row | `{Entity}Row` | `AgentRow`, `EventRow` |
| Storage Create | `Create{Entity}Row` | `CreateEventRow` |
| Storage Update | `Update{Entity}` | `UpdateAgent` |
| Core Domain | `{Entity}` | `Message`, `ContentPart` |
| API Input | `Input{Entity}` | `InputMessage`, `InputContentPart` |
| API Request | `{Action}{Entity}Request` | `CreateMessageRequest` |
| Service | `{Entity}Service` | `AgentService`, `SessionService` |

## Type Flow

```
HTTP:  Controller → Service → Repository → Database
gRPC:  Handler    → Service → Repository → Database
                      ↓
               Core types shared
```

- API receives request DTOs
- Service converts to storage types
- Service returns domain types
- Transport converts to response format

## Error Handling

**API errors:**
- Never expose internal details to clients
- Return generic messages: "Not found", "Invalid request", "Internal server error"
- Always log full error server-side with `tracing::error!()`

```rust
let result = db.operation().await.map_err(|e| {
    tracing::error!("Operation failed: {}", e);
    StatusCode::INTERNAL_SERVER_ERROR
})?;
```

## Testing

### Test Categories

| Category | Location | Dependencies | Run Command |
|----------|----------|--------------|-------------|
| Unit Tests | Inline `#[cfg(test)]` modules | None | `cargo test --lib` |
| Integration Tests | `tests/integration_test.rs` | PostgreSQL, workers | `cargo test --test integration_test` |
| LLM Tests | `tests/agent_run_*.rs` | Real LLM API keys | `cargo test --test agent_run_basic` |

### Unit Tests

Inline `#[cfg(test)]` modules for:
- Serialization/deserialization
- Pure functions
- Business logic without external dependencies

### Integration Tests

`tests/integration_test.rs` for:
- API endpoints
- Multi-service workflows
- Requires infrastructure (PostgreSQL, workers)

### LLM Tests

`crates/core/tests/agent_run_*.rs` for testing against real LLM endpoints:
- `agent_run_basic.rs` - Basic completion + tool calls (Anthropic + OpenAI)
- `agent_run_with_thinking.rs` - Extended thinking tests (Anthropic only)

**Requirements:**
- `ANTHROPIC_API_KEY` and/or `OPENAI_API_KEY` environment variables
- Tests skip gracefully if API keys are not set
- Feature flag: `llm-tests` (enabled by default)

```bash
# Run LLM tests
ANTHROPIC_API_KEY=key OPENAI_API_KEY=key cargo test -p everruns-core --test agent_run_basic
```

### When to Add

- New features: always
- Bug fixes: regression tests
- Security fixes: verification tests

### Running

```bash
just test          # All tests (Rust + UI e2e)
just check         # Format + lint + test
```

## Content Types

`ContentPart` enum across all layers:
- `InputContentPart` - user input (text, image only)
- `ContentPart` - full enum (text, image, tool_call, tool_result)
- `From<InputContentPart> for ContentPart` - safe conversion

## File Organization

**Collocation principle:** Keep related code together.

- API contracts (DTOs) live with their route handlers
- Services in `control-plane/src/services/`
- Storage in `control-plane/src/storage/`
- Core types in `core/src/`

## OpenAPI Support

Types needing OpenAPI schema:
```rust
#[cfg_attr(feature = "openapi", derive(ToSchema))]
```

## UI Patterns

### Entity Select Components

Most entities have an ID (used as value) and a display name. When creating dropdowns for entity selection, create dedicated `{Entity}Select` components that handle the ID-to-name mapping internally.

**Pattern:** `components/{entity}/{entity}-select.tsx`

```tsx
// components/agent/agent-select.tsx
export function AgentSelect({ value, onValueChange, placeholder, includeAll, ... }) {
  const { data: agents = [] } = useAgents();
  const agentMap = useMemo(() => new Map(agents.map(a => [a.id, a])), [agents]);

  return (
    <Select value={value} onValueChange={onValueChange}>
      <SelectTrigger>
        <SelectValue placeholder={placeholder}>
          {value && agentMap.get(value)?.name}
        </SelectValue>
      </SelectTrigger>
      <SelectContent>
        {includeAll && <SelectItem value="all">All</SelectItem>}
        {agents.map((agent) => (
          <SelectItem key={agent.id} value={agent.id}>{agent.name}</SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
```

**Usage:**
```tsx
// ✅ Clean - component handles ID-to-name mapping
<AgentSelect value={agentId} onValueChange={setAgentId} />
<AgentSelect value={agentId} onValueChange={setAgentId} includeAll />

// ❌ Avoid - manual mapping is error-prone
<Select value={agentId} onValueChange={setAgentId}>
  <SelectValue>{agentMap.get(agentId)?.name}</SelectValue>
  ...
</Select>
```

**Existing components:**
- `AgentSelect` - `components/agent/agent-select.tsx`

**Why:** Centralizes the ID-to-name logic, ensures consistency, and prevents the common bug where `SelectValue` shows the ID instead of the name.
