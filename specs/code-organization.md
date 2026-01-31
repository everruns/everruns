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

## Formatting

### Rust

Use `rustfmt` (standard Rust formatter):

```bash
cargo fmt           # Format all Rust code
cargo fmt --check   # Check formatting (CI)
```

### UI (JavaScript/TypeScript)

Use `oxfmt` (Rust-powered formatter from Oxc project):

```bash
cd apps/ui
npm run format        # Format all UI code
npm run format:check  # Check formatting (CI)
```

Config: `apps/ui/oxfmt.json` (100 char width, single quotes, trailing commas)

### Running All Formatters

```bash
just fmt  # Runs cargo fmt + oxfmt + oxlint --fix
```

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

### Test Philosophy

Tests are organized by dependency requirements and execution speed:

1. **Unit tests (pure)** - Fast, no external dependencies, run first for quick feedback
2. **Integration tests (PostgreSQL)** - Test against real database, validate persistence
3. **Workflow tests (Server + Worker)** - End-to-end LLM execution with running services
4. **LLM tests** - Real API calls, require API keys, validate LLM integration

### Test Categories

| Category | CI Job | Dependencies | Speed | Run Command |
|----------|--------|--------------|-------|-------------|
| Unit (Pure) | `unit-test` | None | ~30s | `just test-unit` |
| Integration | `integration-test` | PostgreSQL | ~2min | `just test-integration` |
| Workflow | `workflow-test` | Server + Worker | ~3min | `just test-workflow` |
| CLI E2E | `cli-e2e-test` | Server (DEV_MODE) | ~2min | `./scripts/cli-e2e-test.sh` |
| LLM | (manual) | API keys | varies | `just test-llm` |

### Unit Tests (Pure)

**Goal:** Fast feedback on logic errors without infrastructure.

**Crates tested:**
- `everruns-anthropic` - LLM client SDK, request/response parsing
- `everruns-openai` - LLM client SDK, request/response parsing
- `everruns-internal-protocol` - Protobuf definitions
- `everruns-core` - Agent logic, tool handling, prompt building

Note: `everruns-cli` is binary-only (no lib target) and tested via CLI E2E tests.

**What to test:**
- Serialization/deserialization
- Pure functions
- Business logic without external dependencies
- Error handling paths

```bash
just test-unit  # Runs in ~30s, no Docker needed
```

### Integration Tests (PostgreSQL)

**Goal:** Validate API endpoints and database layer against real PostgreSQL.

**Test files:**
- `crates/server/tests/api_integration_test.rs` - HTTP API tests (in-process, no TCP)
- `crates/server/tests/repository_integration_test.rs` - Direct repository layer tests
- `crates/durable/tests/postgres_integration_test.rs` - Durable execution persistence
- `crates/durable/tests/postgres_repository_test.rs` - Durable SQL operations

**What to test:**
- CRUD operations for all entities
- Database constraints and migrations
- Transaction handling
- Query correctness

```bash
just test-integration  # Starts Docker, runs migrations, tests
```

### Workflow Tests (Server + Worker)

**Goal:** End-to-end validation of LLM workflow execution.

**Test file:** `crates/server/tests/workflow_test.rs`

**What to test:**
- Session creation and message handling
- LLM provider integration
- Worker job execution
- SSE event streaming

**Requirements:**
- Running API server (`everruns-server`)
- Running Worker (`everruns-worker`)
- PostgreSQL database
- `ANTHROPIC_API_KEY` or `OPENAI_API_KEY`

```bash
just start-all        # Start server + worker in another terminal
just test-workflow    # Run workflow tests
```

### LLM Tests

**Goal:** Validate LLM client implementations against real APIs.

**Test files:**
- `crates/core/tests/agent_run_basic.rs` - Basic completion + tool calls
- `crates/core/tests/agent_run_with_thinking.rs` - Extended thinking (Anthropic)

**Requirements:**
- `ANTHROPIC_API_KEY` and/or `OPENAI_API_KEY` environment variables
- Tests skip gracefully if API keys are not set

```bash
just test-llm  # Run against real LLM APIs
```

### CLI E2E Tests

`scripts/cli-e2e-test.sh` tests the CLI binary against a running server:
- Agent CRUD (create, list, get, delete)
- Session CRUD (create, list, get)
- Chat with LLM response verification
- Capabilities listing

**Requirements:**
- Server running at `localhost:9000` (use `just start-dev --no-watch` or DEV_MODE in CI)
- `DEFAULT_OPENAI_API_KEY` or `DEFAULT_ANTHROPIC_API_KEY` for chat tests
- Use `--skip-chat` flag to skip chat tests when no LLM keys available

```bash
# Run CLI e2e tests locally
just start-dev --no-watch &
./scripts/cli-e2e-test.sh

# Skip chat tests (no LLM keys)
./scripts/cli-e2e-test.sh --skip-chat
```

### Test Infrastructure

**In-process API testing:**
- Uses Tower's `ServiceExt::oneshot` for HTTP testing without TCP listener
- See `crates/server/tests/test_harness.rs` for `TestServer` implementation
- Provides fast, isolated tests with full router stack

**Database setup:**
- Tests use `DATABASE_URL` environment variable
- Default: `postgres://everruns:everruns@localhost:5432/everruns_test`
- CI uses PostgreSQL service container
- Local: Docker via `just start-docker`

### When to Add Tests

| Scenario | Required Tests |
|----------|----------------|
| New feature | Unit + Integration |
| Bug fix | Regression test reproducing the bug |
| API endpoint | API integration test |
| Database change | Repository integration test |
| LLM behavior | LLM test (if critical) |

### Running Tests

```bash
just test-unit         # Pure unit tests (~30s)
just test-integration  # PostgreSQL tests (~2min)
just test-workflow     # E2E with server/worker
just test-llm          # Real LLM API tests
just test              # All Rust tests + UI e2e
just check             # Format + lint + test
```

### UI Component Testing

Framework: Jest 30.x with React Testing Library

Test files location: `apps/ui/src/__tests__/`

Configuration: `apps/ui/jest.config.js`

#### Test Patterns

**Hook Mocking:**
```typescript
const mockUseSchedules = jest.fn();
jest.mock("@/hooks/use-durable", () => ({
  useSchedules: () => mockUseSchedules(),
  useCreateSchedule: () => mockUseCreateSchedule(),
}));

// In beforeEach:
mockUseSchedules.mockReturnValue({
  data: { data: mockData, total: 10 },
  isLoading: false,
  error: null,
  refetch: jest.fn(),
});
```

**QueryClient Wrapper:**
```typescript
const wrapper = ({ children }: { children: ReactNode }) => (
  <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
);

// In beforeEach:
queryClient = new QueryClient({
  defaultOptions: {
    queries: { retry: false },
    mutations: { retry: false },
  },
});
```

**Test Data Factories:**
```typescript
function createSchedule(overrides?: Partial<DurableSchedule>): DurableSchedule {
  return {
    id: "sched_123",
    name: "Daily Backup",
    cron_expression: "0 0 0 * * * *",
    timezone: "UTC",
    target: { type: "workflow", name: "backup", input: {} },
    enabled: true,
    ...overrides,
  };
}
```

**Confirm Dialog Mocking:**
```typescript
const mockConfirm = jest.fn();
window.confirm = mockConfirm;

// In test:
mockConfirm.mockReturnValue(true);  // User confirms
mockConfirm.mockReturnValue(false); // User cancels
```

#### Test Categories

| Category | Focus | Example |
|----------|-------|---------|
| State Tests | Loading, error, empty states | `isLoading: true` |
| Render Tests | Correct data display | Badge variants, formatted dates |
| Interaction Tests | User actions | Button clicks, form submission |
| Filter Tests | Search/filter behavior | Query filtering, status filter |
| Dialog Tests | Modal interactions | Open, form fill, submit, close |

#### Running UI Tests

```bash
cd apps/ui
npm test                    # Run all tests
npm test -- schedules       # Run tests matching pattern
npm test -- --coverage      # With coverage report
npm test -- --watch         # Watch mode for development
```

**CI Integration:** UI tests run as part of the `ui-build` job in GitHub Actions CI. Tests must pass before PRs can be merged.

### CI Jobs

| Job | What it tests | Dependencies |
|-----|---------------|--------------|
| `unit-test` | Pure logic, no dependencies | None |
| `integration-test` | PostgreSQL, in-process API | None |
| `build-binaries` | Builds release binaries, uploads artifacts | None |
| `workflow-test` | Server + worker E2E | `build-binaries` |
| `cli-e2e-test` | CLI binary against DEV_MODE server | `build-binaries` |

### CI Caching Strategy

Uses `Swatinem/rust-cache@v2` with shared keys for cross-job cache reuse:

| Shared Key | Used By | Purpose |
|------------|---------|---------|
| `clippy` | clippy | Lint checks |
| `test` | unit-test, integration-test, workflow-test | Test compilation |
| `release` | build-binaries, build, openapi-check | Release builds |

**Artifact sharing:** `build-binaries` uploads server/worker/cli binaries as GitHub artifacts. E2E jobs download pre-built binaries instead of rebuilding (~5 min saved per job).

## Content Types

`ContentPart` enum across all layers:
- `InputContentPart` - user input (text, image only)
- `ContentPart` - full enum (text, image, tool_call, tool_result)
- `From<InputContentPart> for ContentPart` - safe conversion

## File Organization

**Collocation principle:** Keep related code together.

- API contracts (DTOs) live with their route handlers
- Services in `server/src/services/` (control plane)
- Storage in `server/src/storage/` (control plane)
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

### Entity Filter Menu Components

For filtering lists by entity, use menu-style dropdowns instead of form selects. Menus provide better UX for filters (immediate action, no form submission semantics).

**Pattern:** `components/{entity}/{entity}-filter-menu.tsx`

```tsx
// components/agent/agent-filter-menu.tsx
export function AgentFilterMenu({ value, onValueChange, className }) {
  const { data: agents = [] } = useAgents();
  const [open, setOpen] = useState(false);
  const agentMap = useMemo(() => new Map(agents.map(a => [a.id, a])), [agents]);

  const displayValue = value ? agentMap.get(value)?.name : "All agents";
  const handleValueChange = (newValue: string) => {
    onValueChange(newValue);
    setOpen(false); // Close on selection
  };

  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <DropdownMenuTrigger className={cn(buttonVariants({ variant: "outline" }), className)}>
        {displayValue}
        <ChevronDown className="h-4 w-4 ml-2 opacity-50" />
      </DropdownMenuTrigger>
      <DropdownMenuPositioner align="end">
        <DropdownMenuContent>
          <DropdownMenuRadioGroup value={value} onValueChange={handleValueChange}>
            <DropdownMenuRadioItem value="">All agents</DropdownMenuRadioItem>
            {agents.map((agent) => (
              <DropdownMenuRadioItem key={agent.id} value={agent.id}>
                {agent.name}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenuPositioner>
    </DropdownMenu>
  );
}
```

**When to use:**
- `{Entity}Select` - Form inputs (creating/editing records)
- `{Entity}FilterMenu` - List filters (filtering tables/lists)

**Existing components:**
- `AgentFilterMenu` - `components/agent/agent-filter-menu.tsx`

### Entity ID Display

**Rule:** Never show entity IDs as visible text. Provide a copy button for ID access.

**Pattern:**
- Display entity name/title prominently
- Add `CopyButton` next to title for ID copying
- Tooltips may show full ID for reference
- If entity has no name, show `"{EntityType} {shortenId(id)}"` as fallback

**Implementation:**
```tsx
// In headers/titles
<h1 className="text-2xl font-bold flex items-center gap-2">
  {entity.name}
  <CopyButton value={entity.id} />
</h1>

// In cards (within Link)
<div className="flex items-center gap-2">
  <CardTitle>{entity.name}</CardTitle>
  <CopyButton value={entity.id} />
</div>

// Fallback when no name
<h1 className="text-2xl font-bold flex items-center gap-2">
  {entity.name || `Agent ${shortenId(entity.id)}`}
  <CopyButton value={entity.id} />
</h1>
```

**Why:** IDs are for programmatic use (API calls, debugging). Users need to copy them occasionally but don't need to see them constantly. This keeps the UI clean while maintaining accessibility.

**Components:**
- `CopyButton` - `components/ui/copy-button.tsx` - Icon button that copies value to clipboard

### List Page Structure

For pages listing entities with filters:

```
┌─────────────────────────────────────────────┐
│ Page Title                    [Action Btn]  │  ← Simple header
├─────────────────────────────────────────────┤
│ ┌─────────────────────────────────────────┐ │
│ │ Context Title        [Filter Dropdown]  │ │  ← Card header
│ │ X items                                 │ │
│ ├─────────────────────────────────────────┤ │
│ │ Item list...                            │ │  ← Card content
│ │                                         │ │
│ ├─────────────────────────────────────────┤ │
│ │ Showing 1-20 of X     [Prev] [Next]     │ │  ← Pagination
│ └─────────────────────────────────────────┘ │
└─────────────────────────────────────────────┘
```

**Rules:**
- Page header: Simple title + primary action button (no counts)
- Card header: Contextual title (changes with filter) + item count + filter control
- Card content: List items
- Pagination: Shows range and total for current filter

**Why:** Avoids confusion between "total" and "filtered" counts. The card header count always reflects the current view.
