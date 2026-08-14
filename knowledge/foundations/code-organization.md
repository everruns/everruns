---
type: Specification
title: "Code Organization"
description: "Developer conventions: formatting, testing, error handling, UI patterns."
tags:
  - everruns
  - foundations
---
# Code Organization

## Abstract

Developer conventions for formatting, testing, error handling, and UI patterns. For architecture, layer separation, naming conventions, and type flow, see `knowledge/foundations/architecture.md`.

## Crate Documentation

Every workspace crate ships a `README.md` (referenced by `readme = "README.md"`
in its `Cargo.toml`) and a crate-level rustdoc comment (`//!`) on its
`lib.rs`/`main.rs`. Both are user-facing surfaces. For published crates the
README renders on crates.io and the rustdoc on docs.rs; for unpublished
workspace crates the README is read on GitHub and the rustdoc via local
`cargo doc`. Either way they must read as documentation for a newcomer, not as
internal notes.

**Every README must include:**

1. A one-line tagline under the `# crate-name` title.
2. An abstract: one short paragraph stating what the crate is and does.
3. An ecosystem line linking [everruns.com](https://everruns.com) and naming how
   the crate fits (which crates it builds on or pairs with).
4. A quick example — runnable when the crate exposes a usable standalone API;
   a minimal usage snippet for internal or generated crates that cannot run
   standalone.
5. A short "What It Provides" / "Features" list.
6. A "Documentation" section linking the relevant
   [docs.everruns.com](https://docs.everruns.com) page(s), and — for published
   crates — the docs.rs API reference.
7. A "License" section linking the repository
   [`LICENSE`](https://github.com/everruns/everruns/blob/main/LICENSE) (MIT).

**Crate-level rustdoc** should mirror the README's abstract, ecosystem line, and a
compiled example, so docs.rs and crates.io tell the same story. Examples in
`//!` blocks are compiled as doctests and must pass.

**Exclude from both surfaces:**

- Publishing/release mechanics, CI wiring, or "keep this in sync" maintainer notes.
- Links to internal `knowledge/` files (they do not resolve on crates.io or docs.rs);
  reference design intent in code comments instead, or link a published
  docs.everruns.com page.
- Internal "Decision:" scratch notes as the lead content. Reframe durable design
  rationale as a brief, user-facing "Design notes" subsection or move it to
  inline code comments.

**Badges** (crates.io version, docs.rs, license) belong only on crates that are
actually published to crates.io — see the publish list in
`.github/workflows/publish-crates.yml`. A crates.io or docs.rs badge on an
unpublished crate renders as a broken link, so unpublished crates use a
badge-free header and omit docs.rs links.

### Crate dependency conventions

Thin LLM provider crates (`openai`, `anthropic`, `gemini`, `bedrock`, `mai`,
`fireworks`, `openrouter`) depend on `everruns-provider`, **not**
`everruns-core`. `everruns-provider` is the lean provider/LLM abstraction crate
that owns the driver surface (`ChatDriver`, the shared OpenAI/OpenResponses
protocol drivers, model profiles, retry/stream helpers, typed IDs, the
credential form schema, and the LLM error taxonomy). It carries none of core's
hosted subtrees (`a2a` gRPC, knowledge/vector stores, platform delegation), so a
standalone provider build never pulls them in. A provider is therefore a pure
`ChatDriver` implementation with no dependency on core's agent-loop runtime.
Since EVE-874 provider crates carry **no** `everruns-core` edge at all — not
even as a dev-dependency; wire-level tests build their fixtures in-crate, and
`scripts/lib/check-provider-isolation.sh` (pre-push + CI) rejects a direct
core/host/platform/server dependency on any edge kind, plus any heavy core
feature subtree (sqlx/utoipa/inventory/axum/tonic) in provider-only shipped
trees. Default-product registration stays a composition concern: the worker
(`crates/worker/src/adapters.rs`) assembles all official drivers into the
product registry, and downstream providers register through the open
`DriverRegistry` seams alone (see
`tests/fixtures/external-consumer/provider-pack/`).

Deterministic simulation and demo fixtures live in `everruns-test-support`
(EVE-875): the `llmsim` driver, the in-memory agentic loop, mock test doubles,
and the fake/demo capabilities (fake AWS/CRM/financial/warehouse, test
math/weather, sample-data, noop). `everruns-core` carries no llmsim dependency
or feature, product registries never register the fixtures, and
`scripts/lib/check-test-support-isolation.sh` (pre-push + CI) enforces both.
The facade's `Model::simulated` consumes only the crate's `sim` feature.
Reusable writable message/event fixtures also live in test-support. Concrete
application-grade agent, harness, session, and provider stores live in
`everruns-host`; hosted conversation writes append only to its canonical
`EventLog`, with `EventHistory` providing the read-only message projection.

Telemetry initialization and exporter implementations live in
`everruns-observability` (EVE-876): OTLP exporter wiring, tracing-subscriber
layers, `TelemetryConfig`/`init_telemetry`, the `CompositeEventListener`
fan-out, and the OTel/Braintrust exporter listeners. `everruns-core` keeps only
the neutral contracts (the `EventListener` trait, event types, and gen-AI span
conventions) and carries no OpenTelemetry/exporter dependencies; server and
worker binaries install telemetry explicitly through `everruns-observability`.
`scripts/lib/check-observability-isolation.sh` (pre-push + CI) enforces the
boundary and keeps Framework/provider dependency trees exporter-free.

Environment-backed capability implementations live outside `everruns-core`.
Core owns the capability, tool, filesystem, egress, and MCP-neutral contracts;
focused integration crates own filesystem, Bashkit, web fetch, Lua, and
OpenRouter workspace behavior, while `everruns-mcp` owns the MCP capability
adapter and `everruns-http` owns concrete HTTP transports. `everruns-host`
composes feature-selected integrations for embedders, and `everruns-platform`
composes the complete hosted-product catalog. The
`scripts/lib/check-environment-capability-isolation.sh` pre-push/CI guard keeps
the implementation modules and their shell/interpreter/transport dependencies
out of core and the default Framework dependency tree.

Core's per-turn service contracts follow the same ownership rule internally:
loading, provider resolution, tool context, filesystem, session services,
durability, events, images, connections, and delegation each have a focused module. A generic
`traits` module is not a public boundary. Host-only construction contracts,
including `SessionFileSystemFactory`, live in `everruns-host` with the
composition that consumes them.

Hosted capability implementations live in `everruns-platform` (EVE-885):
Knowledge Bases and Indexes, Memories, subagent/agent/A2A delegation,
background and scheduled session work, user hooks, citations, model scouting,
OpenRouter workspace management, and platform-management tools. Core retains
the neutral `Capability`/tool/event/task/delegation contracts, generic
collection hooks, and type-keyed extension bag. Product server/worker/local
presets compose the hosted registry explicitly; the Framework preset does not
advertise service-backed product capabilities. The capability-isolation guard
keeps these implementations and their service contracts out of core.

The session-service capabilities followed (EVE-886): `session`,
`session_storage`, `session_sql_database` and `session_sandbox` each need a host
service — a session store, key/value plus secret storage, a SQL database, or a
sandbox provider — so they sit with the other service-backed families in
`everruns-platform`. Composition, not the kernel, decides what a deployment
advertises: product registration installs all four (sandbox behind its feature
flag), and the host preset re-adds the two the default in-process runtime can
serve, so the Framework still exposes session info and session storage.

The records those capabilities read follow them (EVE-880). The org-scoped
`Workspace` row and the managed per-session sandbox — state record, provider
SPI, inventory plugin and lifecycle helpers — live in `everruns-platform`; a
turn resolves workspace *paths* through core's roots and policy types, and
reaches a sandbox only through the capability. `session_sqldb` is the
deliberate exception: its value types are the signature vocabulary of
`SessionSqlDbStore`, which `ToolContext` still holds as an optional field, so
records and trait move together under EVE-887 rather than leaving core naming
platform types. Session task, schedule and resource records likewise stay
while core execution still consumes them.

`everruns-core` depends on `everruns-provider` with default features disabled
and imports only the contracts needed by neutral execution. It does not
re-export provider-owned modules. Low-level consumers declare
`everruns-provider` directly; the application-facing `everruns` facade may
selectively expose the coherent Framework API from either owner. Crates that
pull in the host (for example `everruns-local` → `everruns-host`) still depend
on full `everruns-core`.

Two pin conventions follow from the publish set:

- **Unpublished** crates reference path deps without a version
  (`{ path = "../provider" }` for provider crates; `{ path = "../core", default-features = false }` for crates that do depend on core), matching `server`/`worker`.
- **Published** crates that pin a sibling by exact version must be registered in
  *both* `.github/workflows/publish-crates.yml` (`dependency_versions`) and
  `scripts/sync-publish-pin-versions.py` (`INNER_PINS`). The release process
  bumps pins via the latter; the publish workflow rejects releases where a pin
  drifts. Adding a published crate to only one list breaks releases.

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
pnpm run format        # Format all UI code
pnpm run format:check  # Check formatting (CI)
```

Config: `apps/ui/oxfmt.json` (100 char width, single quotes, trailing commas)

### Running All Formatters

```bash
just fmt  # Runs cargo fmt + oxfmt + oxlint --fix
```

### Policy Enforcement

Every non-GET `Command` impl MUST declare `fn policy() -> Option<&'static Policy>`. The command runner (`Command::run`) evaluates the declared policy against the `PermissionResolver` threaded through `Ctx` before dispatching to `execute`. Use `Caller::internal(org_id)` for gRPC/internal paths. See `knowledge/security/permissions.md` for the full policy model.

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
| Shell (Repo Scripts) | `shell-test` | Bash + repo script deps | ~10s | `just test-shell` |
| Unit (Pure) | `unit-test` | None | ~30s | `just test-unit` |
| Integration | `integration-test` | PostgreSQL | ~2min | `just test-integration` |
| Workflow | `workflow-test` | Server + Worker | ~3min | `just test-workflow` |
| CLI E2E | `cli-e2e-test` | Server (DEV_MODE) | ~2min | `./scripts/cli-e2e-test.sh` |
| LLM | (manual) | API keys | varies | `just test-llm` |

`integration-test` is path-filtered in CI and runs when PostgreSQL-backed
surfaces, provider live-test crates bundled into that job, workspace dependency
files, the Rust toolchain pin, or the CI workflow itself change.

### Unit Tests (Pure)

**Goal:** Fast feedback on logic errors without infrastructure.

**Crates tested:**
- `everruns-anthropic` - LLM client SDK, request/response parsing
- `everruns-openai` - LLM client SDK, request/response parsing
- `everruns-internal-protocol` - Protobuf definitions
- `everruns-core` - Agent logic, tool handling, prompt building
- `everruns-host` - in-process runtime and shared host-phase orchestration
- `everruns-cli` - subprocess integration tests for help text, auth profile handling, and file-sync argument validation

Note: `everruns-cli` is binary-only (no lib target). Keep its lightweight subprocess
coverage in `crates/cli/tests/*.rs`; use CLI E2E tests for full round-trip flows
against a running server.

**What to test:**
- Serialization/deserialization
- Pure functions
- Business logic without external dependencies
- Error handling paths

```bash
just test-unit  # Runs in ~30s, no Docker needed
```

### Integration Tests (PostgreSQL)

**Goal:** Validate API endpoints, repository layer, and durable execution against real PostgreSQL.

**Test files:**
- `crates/server/tests/api_integration_test.rs` - HTTP API tests (in-process, no TCP)
- `crates/server/tests/repository_conformance_test.rs` - Shared PostgreSQL/in-memory storage contract tests
- `crates/server/tests/repository_integration_test.rs` - Direct repository layer tests
- `crates/server/tests/ag_ui_integration_test.rs` - AG-UI embedding + publish gating
- `crates/server/tests/auth_integration_test.rs` - Refresh/revocation, cookie flags, JWT paths
- `crates/server/tests/cli_auth_test.rs` - CLI login flow (start/callback/success)
- `crates/server/tests/cli_auth_no_org_test.rs` - CLI exchange rejects no-org users (EVE-195)
- `crates/server/tests/client_side_tools_test.rs` - Tool call / result wire format
- `crates/server/tests/evals_integration_test.rs` - Eval case/score/run HTTP surface
- `crates/server/tests/llm_model_default_test.rs` - Org default model upsert/clear
- `crates/server/tests/org_creation_test.rs` - Organization + membership reconciliation
- `crates/server/tests/org_isolation_test.rs` - Cross-org isolation (agents, mcp, models, sessions)
- `crates/server/tests/org_lifecycle_test.rs` - Switch org, cookie scoping, unknown org rejection
- `crates/server/tests/schedule_integration_test.rs` - Scheduled-task CRUD + pause/resume
- `crates/server/tests/session_git_integration_test.rs` - Per-session git log/diff/refs
- `crates/durable/tests/postgres_integration_test.rs` - Durable execution task queue, workflows
- `crates/durable/tests/postgres_repository_test.rs` - Durable SQL queries, circuit breakers
- `crates/durable/tests/failure_injection_test.rs` - fail-rs rollback and retry coverage (`failpoints,postgres-tests`)
- `crates/durable/tests/agent_reliability_test.rs` - manual executor/store reliability harness; not CI-gated until the restart/reclaim scenarios pass cleanly

> Not yet CI-wired: `mcp_endpoint_test.rs` and `skills_integration_test.rs` fail
> due to shared-fixture name collisions and stale catalog expectations; tracked
> as follow-ups. `workflow_test.rs` requires a live server+worker and stays a
> manual/live test.

**What to test:**
- CRUD operations for all entities
- Database constraints and migrations
- Transaction handling
- Query correctness
- Soft-delete behavior (agents archived, not hard-deleted)
- Append-only constraints (events cannot be deleted)

**Durable test requirements:**
- Tests using `claim_task` must register workers first via `register_test_worker()`
- Workers must be cleaned up via `cleanup_worker()` at test end
- This is required because `claim_task` SQL joins against `durable_workers` table

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
- `crates/llm-tests/tests/agent_run_basic.rs` - Basic completion + tool calls
- `crates/llm-tests/tests/agent_run_with_thinking.rs` - Extended thinking (Anthropic)

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
- Server running at `localhost:9300` (use `just start-dev --no-watch` or DEV_MODE in CI)
- `DEFAULT_OPENAI_API_KEY` or `DEFAULT_ANTHROPIC_API_KEY` for chat tests
- Use `--skip-chat` flag to skip chat tests when no LLM keys available

```bash
# Run CLI e2e tests locally
just start-dev --no-watch &
./scripts/cli-e2e-test.sh

# Skip chat tests (no LLM keys)
./scripts/cli-e2e-test.sh --skip-chat
```

### Shell Helper Tests

Repo-owned shell-layer tests live in `scripts/test-*.sh` and run through the
shared `just test-shell` / `scripts/run-shell-tests.sh` entrypoint. Keep these
tests in shell when they need to exercise shell libraries, git config/env
behavior, port-prefix expansion, or process-control scripts directly.

Scope:
- `scripts/test-*.sh` covers repository shell libraries and helper behavior
- `.github/scripts/**` stays workflow-specific and is validated by the CI jobs
  that invoke those scripts directly

```bash
just test-shell
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
- Local: `just start-infra` (native pg_ctl + valkey)

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
just test-shell        # Repo shell helper tests
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
pnpm test                    # Run all tests
pnpm test -- schedules       # Run tests matching pattern
pnpm test -- --coverage      # With coverage report
pnpm test -- --watch         # Watch mode for development
```

**CI Integration:** UI tests run as part of the `ui-build` job in GitHub Actions CI. Tests must pass before PRs can be merged.

### UI Playwright Smoke

Framework: Playwright (`@playwright/test`), chromium-only.

Test files location: `apps/ui/e2e/`

Configuration: `apps/ui/playwright.config.ts`

**Scope:** PR-gated smoke coverage of shared chat primitives via the `/dev/*` reference gallery pages. These pages render without a running backend (pure Next.js, no API calls), which keeps the suite stable in CI. Product flows requiring authentication and backend state remain out of scope until we add dedicated test fixtures.

**Layering (when to put a test where):**

- Jest + React Testing Library — component-level behavior, hook mocking, dialog interactions.
- Playwright smoke — end-to-end rendering of shared primitives as composed pages; catches regressions that unit-rendered components do not (routing, SSR/CSR hydration, CSS load, provider wiring).
- Durable workflow tests (Rust `workflow-test`) — backend-side end-to-end flows.

**CI Integration:** runs as the `ui-e2e` job on every PR where `apps/ui/**` changes. The job installs Playwright chromium, starts `pnpm run dev` via the Playwright webServer config, and executes `pnpm run e2e`.

```bash
cd apps/ui
pnpm run e2e          # Run all Playwright tests
pnpm run e2e:ui       # Run with UI mode
pnpm run e2e:headed   # Run in headed mode
```

### CI Jobs

| Job | What it tests | Dependencies |
|-----|---------------|--------------|
| `unit-test` | Pure logic, no dependencies | None |
| `worker-test` | Worker durable execution lib tests | None |
| `integration-test` | PostgreSQL, in-process API | None |
| `build-binaries` | Builds release binaries, uploads artifacts | None |
| `workflow-test` | Server + worker E2E | `build-binaries` |
| `cli-e2e-test` | CLI binary against DEV_MODE server | `build-binaries` |
| `ui-build` | UI format, lint, Jest, Next.js build | None |
| `ui-e2e` | Playwright smoke against `/dev/*` reference pages | None |

### CI Caching Strategy

Uses `Swatinem/rust-cache@v2` with shared keys for cross-job cache reuse:

| Shared Key | Used By | Purpose |
|------------|---------|---------|
| `clippy` | clippy | Lint checks |
| `test` | unit-test, integration-test, workflow-test | Test compilation |
| `release` | build-binaries, build, openapi-check | Release builds |

**Artifact sharing:** `build-binaries` uploads server/worker/cli binaries as GitHub artifacts. E2E jobs download pre-built binaries instead of rebuilding (~5 min saved per job).

## UI Patterns

### Dropdowns Must Show Display Names

**Rule:** Every dropdown (Select/Combobox) must show the human-readable display name of the selected option, never the raw internal value (e.g. `"Webhook"`, not `"webhook"`; `"Shared Session"`, not `"shared_session"`).

**How it works:** The shared `<Select>` wrapper at `components/ui/select.tsx` walks its `<SelectItem>` children at render time and forwards a `value → label` map to base-ui's `items` prop. base-ui's `<SelectValue>` then resolves the label automatically. Callers do not need to do anything special — just render `<SelectItem value="..."><Label/></SelectItem>` and `<SelectValue />`.

**Authoring rules:**
- Always put a human-readable label in `<SelectItem>` children — never just the raw value.
- Use `<SelectValue />` (no children, no manual lookup) for the common case. The wrapper resolves the label.
- Pass an explicit `<SelectValue>{node}</SelectValue>` only when composing extra trigger content (e.g. icon + label).
- Auto-collection traverses the JSX tree passed as `<Select>` children — it does **not** render custom wrapper components that internally return a `<SelectItem>` (e.g. `<MySelectItem />`). For such cases, or for options that aren't visible as static JSX children of `<Select>` (lists built far from the root, portals, async loads), pass an explicit `items={{ value: label, ... }}` prop on `<Select>`.
- For combobox/autocomplete, options must be `{ value, label }` pairs and the trigger must show `label`.

**Why:** Internal values like `shared_session` or `webhook` leak implementation details into the UI and look unprofessional. Centralising the resolution in the shared wrapper makes "display name" the default and prevents the bug from recurring.

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
- `CopyButton` - `components/ui/copy-button.tsx` - Hash (`#`) icon button that copies value to clipboard. Uses `Hash` icon (not generic `Copy`) to visually distinguish "copy ID" from other copy actions (e.g., URL copy buttons). Shows green `Check` icon after copying.

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

### Page Titles

Every route under `apps/ui/src/app/` MUST set a meaningful `<title>`. Browser
tabs, history, and screen readers rely on it — a single static title across
every page is not acceptable.

**Format:** `<Specific> · <Section> · Everruns`

- Separator: middle dot ` · ` (U+00B7) with single spaces.
- App suffix: always `Everruns` at the end.
- Order: most specific first, broadest last (browser tabs truncate from the
  right, so the distinguishing bit must lead).
- Two to four segments. Drop empty segments.

Examples:

- `Agents · Everruns` (list)
- `New Agent · Agents · Everruns` (create)
- `My Customer Bot · Agent · Everruns` (detail)
- `Edit · My Customer Bot · Agent · Everruns` (edit)
- `API Keys · Settings · Everruns` (settings sub-tab)
- `Trajectory · Friday triage · Session · Everruns` (session sub-tab)
- `Sign in · Everruns` (auth)

**Display name vs name:** for entities with both `name` and `display_name`,
the title MUST use `getDisplayName(entity)` from
`apps/ui/src/lib/entity-lifecycle.ts`. The slug `name` is for URLs, never the
title. While the entity is still loading, fall back to the kind alone
(`Agent · Everruns`) — the title updates in place once data resolves.

**Implementation:** `apps/ui/src/lib/page-title.ts` exports `formatPageTitle()`
and `apps/ui/src/hooks/use-page-title.ts` exports `usePageTitle()`. The hook
sets `document.title` via `useEffect` and restores the previous title on
unmount; `null` / `undefined` segments are skipped so loading states work.
Server pages with no client interactivity SHOULD export Next.js `metadata`
instead. Mixing both is fine — the hook overrides static metadata once
mounted.

**Coverage by page type:**

- List/index: section name (`Agents`)
- Create: `New <Kind>`, with the section as a second segment
- Edit: prefix `Edit · ` to the detail title
- Detail: `<displayName> · <Kind>`
- Sub-tabs (sessions, settings, durable): `<Sub> · <Parent specific or kind>`
- Auth: action verb only (`Sign in`, `Sign up`)
- Dev / showcase: `<Component> · Dev` (and `Page not found` when the dev guard
  hides the route in production)
- Error / not-found boundaries (`error.tsx`, `not-found.tsx`): set a title
  matching the rendered fallback (e.g. `Page not found`,
  `Something went wrong`) so tabs do not advertise a nonexistent route

When adding a new route, the page title is part of the work — pages without a
title should fail review.
