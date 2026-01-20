## Coding-agent guidance (repo root)

This repo is intended to be runnable locally and easy for coding agents to work in.

### Style

Telegraph. Drop filler/grammar. Min tokens (global AGENTS + replies).

### Critical Thinking

Fix root cause (not band-aid). Unsure: read more code; if still stuck, ask w/ short options. Unrecognized changes: assume other agent; keep going; focus your changes. If it causes issues, stop + ask user. Leave breadcrumb notes in thread.

### Principles

- Keep decisions as comments on top of the file. Only important decisions that could not be inferred from code.
- Code should be easily testable, smoke testable, runnable in local dev env.
- Prefer small, incremental PR-sized changes with a runnable state at each step.
- Avoid adding dependencies with non-permissive licenses. If a dependency is non-permissive or unclear, stop and ask the repo owner.
- No backward compatibility needed. All code is internal; no external consumers.

### Specs

`specs/` folder contains feature specifications outlining requirements for specific features and components. New code should comply with these specifications or propose changes to them.

Available specs:
- `specs/architecture.md` - System architecture, crate structure, infrastructure
- `specs/models.md` - Data models (Agent, Session, Message, etc.)
- `specs/apis.md` - HTTP API endpoints
- `specs/events.md` - Event types and SSE streaming
- `specs/tool-execution.md` - Tool types and execution flow
- `specs/capabilities.md` - Agent capabilities system for modular functionality
- `specs/mcp-servers.md` - MCP (Model Context Protocol) server registration
- `specs/llm-drivers.md` - LLM driver trait, error types, provider implementations
- `specs/durable-execution-engine.md` - PostgreSQL-backed durable workflow engine
- `specs/authentication.md` - Authentication modes and OAuth integration
- `specs/encryption.md` - Envelope encryption for sensitive data
- `specs/session-filesystem.md` - Per-session virtual filesystem
- `specs/usage-tracking.md` - LLM token usage tracking and aggregation
- `specs/documentation.md` - Documentation site (Astro Starlight, Cloudflare Pages)
- `specs/brand.md` - Brand identity, colors, typography, voice
- `specs/dismissed-options.md` - Technical options considered but dismissed
- `specs/multitenancy.md` - Organization-based multitenancy and resource isolation
- `specs/release-process.md` - Release workflow with CHANGELOG.md as source of truth

Specification format: Abstract and Requirements sections.

### Skills

`.claude/skills/` contains development skills following the [Agent Skills Specification](https://github.com/anthropics/skills/blob/main/spec/agent-skills-spec.md).

Available skills:
- `smoke-test/` - API and UI smoke testing with Docker-based environment
- `no-docker-setup/` - Deterministic PostgreSQL setup for cloud agents and CI systems
- `ui-screenshots/` - Take UI screenshots using Playwright and attach them as PR comments

### Test Cases

`test_cases/` contains manual test case documentation organized by feature. Each feature has its own folder with individual test case files.

Test case format:
- **Description**: What the test verifies
- **Preconditions**: Required setup and environment configuration
- **Test Data**: Input values in table format
- **Steps**: Numbered actions to perform
- **Expected Result**: Success criteria

Naming convention: `TC###_short_description.md` (e.g., `TC001_success_login.md`)

When adding new features, create corresponding test cases to document expected behavior and acceptance criteria.

### Public Documentation

Documentation is published at https://docs.everruns.com/ via Cloudflare Pages.

**Source locations:**
- `docs/` - Source markdown files (edit docs here)
- `apps/docs/` - Astro Starlight documentation site (reads from `docs/` via symlink)

**Content structure:**
- `docs/getting-started/` - Quickstart guides
- `docs/features/` - Feature documentation
- `docs/sre/` - SRE documentation
  - `environment-variables.md` - Configuration environment variables
  - `admin-container.md` - Admin container usage guide
  - `runbooks/` - Operational runbooks for common tasks
- `docs/api/` - API reference documentation (auto-generated from OpenAPI spec)

**API Reference documentation:**

The API reference is auto-generated from `docs/api/openapi.json` using `starlight-openapi`. To update the spec:

```bash
# Export the OpenAPI spec (no running server required)
./scripts/export-openapi.sh

# Verify the docs build
cd apps/docs && npm run build
```

The spec should be regenerated and committed whenever API endpoints change.

**Development:**
```bash
cd apps/docs
npm install
npm run dev      # Local development server
npm run check    # Type checking
npm run build    # Production build
```

When making changes that affect user-facing behavior or operations, update the relevant docs in `docs/`. Ensure the docs build passes before creating a PR.

### Local dev expectations

**Command runner options:**
- `./scripts/dev.sh <command>` - Bash script (always available)
- `just <command>` - [just](https://github.com/casey/just) command runner (optional, install: `cargo install just`)

Both are equivalent; `just` wraps `dev.sh`. Use whichever you prefer.

**Full mode (with PostgreSQL):**
- A `local/docker-compose.yml` brings up Postgres + Jaeger

**DEV_MODE (no database required):**
```bash
# Quick start - no Docker/PostgreSQL needed
./scripts/dev.sh start-dev  # or: just start-dev
```
- Uses in-memory storage (data lost on restart)
- Execution happens in-process (no separate worker)
- Ideal for UI development and quick API testing
- See `docs/sre/environment-variables.md` for details

### Smoke test prerequisites

Before running smoke tests, ensure these tools are installed:

1. **PostgreSQL 17** - Database for storing agents, sessions, messages
   - Debian/Ubuntu: `apt-get install postgresql-17`
   - macOS: `brew install postgresql@17`
   - Verify: `psql --version`

2. **jq** - JSON processor for test scripts
   - Debian/Ubuntu: `apt-get install jq`
   - macOS: `brew install jq`

**For Docker environments:** Use `.claude/skills/smoke-test/` (Docker handles PostgreSQL)

**For no-Docker environments:** Use `.claude/skills/no-docker-setup/` (deterministic PostgreSQL setup)

### Cloud Agent environments

When running in cloud-hosted agent environments (e.g., Claude Code on the web), the following secrets are available:

- `OPENAI_API_KEY`: Available for LLM-related operations (OpenAI models)
- `ANTHROPIC_API_KEY`: Available for LLM-related operations (Claude models)
- `GITHUB_TOKEN`: Available for GitHub API operations (PRs, issues, repository access)

These secrets are pre-configured in the environment and do not require manual setup.

If `gh` tool is not available, use GitHub API with `GITHUB_TOKEN`.

### Rust conventions

- Use stable Rust (edition 2024) and keep the toolchain pinned via `rust-toolchain.toml`.
- Run `cargo fmt` and `cargo clippy -- -D warnings` for touched crates.
- Prefer `axum`/`tower` for HTTP, `sqlx` for Postgres, `serde` for DTOs.

### Code organization conventions

The codebase follows a layered architecture with clear boundaries. See `specs/architecture.md` for full details.

#### Layer separation

1. **Storage Layer** (`control-plane/src/storage/`):
   - Database models use `Row` suffix: `AgentRow`, `SessionRow`, `EventRow`
   - Create input structs: `CreateAgentRow`, `CreateEventRow`
   - Update structs: `UpdateAgent`, `UpdateSession`
   - Repositories handle raw database operations only
   - Migrations in `control-plane/migrations/`
   - Note: Messages are stored as events (see `specs/models.md`)

2. **Core Layer** (`core/` → `everruns-core`):
   - Source of truth for all shared data structures
   - Domain types: `Agent`, `Session`, `Message`, `Event`, `ContentPart`
   - Tool types: `ToolCall`, `ToolResult`, `ToolDefinition`
   - Trait definitions: `MessageRetriever`, `EventEmitter`, `LlmProvider`
   - Types are DB-agnostic and serializable
   - OpenAPI support via feature flag: `#[cfg_attr(feature = "openapi", derive(ToSchema))]`

3. **Control-Plane Layer** (`control-plane/` → `everruns-control-plane`):
   - HTTP API (axum) on port 9000, gRPC server (tonic) on port 9001
   - API contracts collocated with routes (e.g., `messages.rs` has routes + DTOs)
   - Services accept API DTOs, transform to storage types, store in database
   - Input types: `InputMessage`, `InputContentPart` (for user-facing input)
   - Request wrappers: `CreateMessageRequest`, `CreateAgentRequest`

4. **Internal Protocol Layer** (`internal-protocol/` → `everruns-internal-protocol`):
   - gRPC protocol definitions (proto files) for worker ↔ control-plane
   - Generated Rust types via tonic-build + protox (pure Rust, no external protoc binary)
   - Batched operations: `GetTurnContext`, `EmitEventStream`

#### Naming conventions

| Layer | Pattern | Example |
|-------|---------|---------|
| Storage Row | `{Entity}Row` | `AgentRow`, `EventRow` |
| Storage Create | `Create{Entity}Row` | `CreateEventRow` |
| Storage Update | `Update{Entity}` | `UpdateAgent` |
| Core Domain | `{Entity}` | `Message`, `ContentPart` |
| API Input | `Input{Entity}` | `InputMessage`, `InputContentPart` |
| API Request | `{Action}{Entity}Request` | `CreateMessageRequest` |

#### Type flow

```
API Request → API DTO → Service → Storage Row → Database
                ↓
         Core types (shared)
```

For example, creating a message:
1. API receives `CreateMessageRequest` with `InputMessage`
2. Service converts `InputContentPart[]` → `ContentPart[]` (core types)
3. Service creates `CreateEventRow` with message data as JSON
4. Repository stores event to database (messages stored as events)

#### Content types

Message content uses unified `Vec<ContentPart>` across all layers:
- `ContentPart` - Full enum: text, image, tool_call, tool_result
- `InputContentPart` - Restricted for user input: text, image only
- `From<InputContentPart> for ContentPart` - Safe conversion

### API error handling

- **Never expose internal error details to API clients.** Database errors, connection failures, and other internal errors must return a generic `500 Internal Server Error` with message "Internal server error".
- **Always log the full error server-side.** Use `tracing::error!()` to log the complete error details before returning the generic response.
- Error messages returned to clients should only contain safe, user-facing information (e.g., "Not found", "Invalid request", "Internal server error").
- Example pattern:
  ```rust
  let result = state.db.some_operation().await.map_err(|e| {
      tracing::error!("Failed to perform operation: {}", e);
      StatusCode::INTERNAL_SERVER_ERROR
  })?;
  ```

### CI expectations

- CI is implemented using GitHub Actions, status is available via `gh` tool
- **NEVER merge when CI is red.** This is a hard requirement with no exceptions. Even if the failure appears unrelated to your changes, do not merge. Fix the issue or get help first.

### Pre-PR checklist

Before creating a pull request:

1. **Run pre-PR script**: `./scripts/dev.sh pre-pr` or `just pre-pr` (runs checks 2-8 automatically)
2. **Rust formatting**: `cargo fmt --check`
3. **Rust linting**: `cargo clippy --all-targets --all-features -- -D warnings`
4. **Rust tests**: `cargo test --all-features`
5. **UI lint**: `npm run lint` in `apps/ui/`
6. **UI build**: `npm run build` in `apps/ui/`
7. **OpenAPI spec freshness**: Verifies `docs/api/openapi.json` matches current code
8. **Docs build**: `npm run check && npm run build` in `apps/docs/`
9. **Rebase on main**: Branch must be rebased on latest `main` before merging
   ```bash
   git fetch origin main && git rebase origin/main
   ```
10. **Smoke tests**: New functionality must be smoke tested end-to-end
11. **UI screenshots**: If PR includes UI changes, attach screenshot of working UI
    - Use real backend data (not faked), except for `/dev/*` component showcase pages
    - Use `.claude/skills/ui-screenshots/` to capture screenshots via Playwright
12. **CI green**: All CI checks must pass before merging
13. **PR comments resolved**: No unaddressed review comments in PR
14. **Examples**: If modifying examples, validate they run against a running API
15. **Update specs**: If changes affect system behavior, update specs in `specs/`
16. **Update docs**: If changes affect usage, update docs in `docs/`

CI will fail if any automated checks fail. Always run `./scripts/dev.sh pre-pr` (or `just pre-pr`) before pushing.

### UI conventions

- Use **npm** for package management (CI uses `npm ci`)
- After adding dependencies, ensure `package-lock.json` is updated via `npm install`
- Run `npm run lint` to check for oxlint issues
- Run `npm run build` to verify TypeScript types and build before pushing

**E2E Testing (Playwright):**
- Tests located in `apps/ui/e2e/`
- Run with `./scripts/dev.sh e2e` or `npm run e2e` in apps/ui
- Screenshot tests: `./scripts/dev.sh e2e-screenshots`
- Dev pages (`/dev/*`) provide component showcases for visual testing

### Testing conventions

PRs should include appropriate tests for the changes being made:

- **Unit tests**: Include inline `#[cfg(test)]` modules for:
  - Data structure validation (serialization/deserialization)
  - Error response formats
  - Pure functions and transformations
  - Business logic that doesn't require external dependencies

- **Integration tests**: Add to `tests/integration_test.rs` for:
  - New API endpoints
  - Complex workflows spanning multiple services
  - End-to-end functionality verification

- **When to add tests**:
  - New features: Always include tests
  - Bug fixes: Add regression tests when feasible
  - Refactoring: Ensure existing tests still pass; add tests if coverage gaps are found
  - Security fixes: Include tests verifying the fix (e.g., error messages don't leak internal details)

- **Running tests**:
  ```bash
  # Unit tests (no external dependencies)
  cargo test

  # Integration tests (requires API + Worker running)
  cargo test -p everruns-control-plane --test integration_test -- --test-threads=1
  ```

### Commit message conventions

Follow [Conventional Commits](https://www.conventionalcommits.org) for all commit messages:

```
<type>[optional scope]: <description>

[optional body]

[optional footer(s)]
```

**Types:**
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `style`: Code style (formatting, semicolons, etc.)
- `refactor`: Code refactoring without feature/fix
- `perf`: Performance improvements
- `test`: Adding or updating tests
- `chore`: Build process, dependencies, tooling
- `ci`: CI configuration changes

**Examples:**
```
feat(api): add agent versioning endpoint
fix(workflow): handle timeout in run execution
docs: update API documentation
refactor(db): simplify connection pooling
```

**Validation (optional):**
```bash
# Validate a commit message
echo "feat: add new feature" | npx commitlint

# Validate last commit
npx commitlint --from HEAD~1 --to HEAD
```

### PR (Pull Request) conventions

PR titles should follow Conventional Commits format. Use the PR template (`.github/pull_request_template.md`) for descriptions.

**Merge strategy**: Always use **Squash and Merge** via GitHub when merging PRs to `main`. This keeps the main branch history clean with one commit per PR.

**PR Body Template:**

```markdown
## What
Clear description of the change.

## Why
Problem or motivation.

## How
High-level approach.

## Risk
- Low / Medium / High
- What can break

## Checklist
- [ ] Tests added or updated
- [ ] Backward compatibility considered
```

## Testing the system

The best way to verify the system is working is to run the **smoke test script**, which tests the full workflow including agent creation, sessions, messages, workflow execution, and optionally the UI:

```bash
# First-time setup (installs Rust tools + UI dependencies)
./scripts/dev.sh init       # or: just init

# Option 1: Start everything at once
./scripts/dev.sh start-all  # or: just start-all

# Option 2: Start services individually
./scripts/dev.sh start      # Start Docker services (or: just start)
./scripts/dev.sh migrate    # Run migrations (or: just migrate)
./scripts/dev.sh api        # Start API (or: just api)
./scripts/dev.sh worker     # Start worker (or: just worker)
./scripts/dev.sh ui         # Start UI (or: just ui)

# Run smoke tests - see .claude/skills/smoke-tests/SKILL.md for test checklist
```

Expected output:
- ✅ Health check passes
- ✅ Agent CRUD operations work
- ✅ Sessions and messages work
- ✅ Session status transitions: pending → running → pending (cycles)
- ✅ OpenAPI spec is available
- ✅ UI pages load correctly

### Alternative testing methods

**Integration tests** (requires API + Worker running):
```bash
cargo test -p everruns-control-plane --test integration_test -- --test-threads=1
```

**Examples** (requires API running):
```bash
cargo run --example create_agent
```

**Manual testing**:
- API docs: http://localhost:9000/swagger-ui/
- UI: http://localhost:9100
- Health check: `curl http://localhost:9000/health`


### Project Structure

```
everruns/
├── apps/
│   ├── ui/               # Next.js Management UI
│   └── docs/             # Astro Starlight Documentation Site
├── crates/
│   ├── control-plane/    # HTTP API + gRPC server + database layer (everruns-control-plane)
│   ├── worker/           # Durable worker with gRPC client (everruns-worker)
│   ├── core/             # Core abstractions, domain entities, tools (everruns-core)
│   ├── internal-protocol/ # gRPC protocol definitions (everruns-internal-protocol)
│   ├── openai/           # OpenAI provider (everruns-openai)
│   └── anthropic/        # Anthropic provider (everruns-anthropic)
├── docs/                 # Documentation content (published via apps/docs)
├── local/              # Docker Compose
├── specs/                # Specifications
└── scripts/              # Dev scripts
```

