## Coding-agent guidance (repo root)

This repo is intended to be runnable locally and easy for coding agents to work in.

### Principles

- Keep decisions as comments on top of the file. Only important decisions that could not be inferred from code.
- Code should be easily testable, smoke testable, runnable in local dev env.
- Treat Temporal as an internal implementation detail behind a small adapter boundary.
- Prefer small, incremental PR-sized changes with a runnable state at each step.
- Avoid adding dependencies with non-permissive licenses. If a dependency is non-permissive or unclear, stop and ask the repo owner.

### Specs

`specs/` folder contains feature specifications outlining requirements for specific features and components. New code should comply with these specifications or propose changes to them.

Available specs:
- `specs/architecture.md` - System architecture, crate structure, infrastructure
- `specs/models.md` - Data models (Agent, Thread, Run, etc.)
- `specs/apis.md` - HTTP API endpoints
- `specs/tool-execution.md` - Tool types and execution flow
- `specs/capabilities.md` - Agent capabilities system for modular functionality
- `specs/documentation.md` - Documentation site (Astro Starlight, Cloudflare Pages)
- `specs/brand.md` - Brand identity, colors, typography, voice
- `specs/dismissed-options.md` - Technical options considered but dismissed

Specification format: Abstract and Requirements sections.

### Skills

`.claude/skills/` contains development skills following the [Agent Skills Specification](https://github.com/anthropics/skills/blob/main/spec/agent-skills-spec.md).

Available skills:
- `smoke-tests/` - API and UI smoke testing with support for Docker and no-Docker environments

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
- `docs/api/` - API reference documentation

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

- A `harness/docker-compose.yml` brings up Temporal + Postgres + required dependencies
- `protoc` (Protocol Buffers compiler) is required for building Temporal SDK dependencies
  - Debian/Ubuntu: `apt-get install protobuf-compiler`
  - macOS: `brew install protobuf`
  - Or download from https://github.com/protocolbuffers/protobuf/releases

### Smoke test prerequisites

Before running smoke tests, ensure these tools are installed:

1. **PostgreSQL** - Database for storing agents, sessions, messages
   - Debian/Ubuntu: `apt-get install postgresql postgresql-contrib`
   - macOS: `brew install postgresql`
   - Verify: `psql --version`

2. **Temporal CLI** - Workflow orchestration server
   - Install: `curl -sSf https://temporal.download/cli.sh | sh`
   - Add to PATH: `export PATH="$PATH:$HOME/.temporalio/bin"`
   - Verify: `temporal --version`

3. **protoc** - Protocol Buffers compiler (see above)

4. **jq** - JSON processor for test scripts
   - Debian/Ubuntu: `apt-get install jq`
   - macOS: `brew install jq`

The no-Docker smoke test scripts (`.claude/skills/smoke-test/scripts/`) can auto-install some of these, but having them pre-installed speeds up testing.

### Cloud Agent environments

When running in cloud-hosted agent environments (e.g., Claude Code on the web), the following secrets are available:

- `OPENAI_API_KEY`: Available for LLM-related operations (OpenAI models)
- `ANTHROPIC_API_KEY`: Available for LLM-related operations (Claude models)
- `GITHUB_TOKEN`: Available for GitHub API operations (PRs, issues, repository access)

These secrets are pre-configured in the environment and do not require manual setup.

### Rust conventions

- Use stable Rust (edition 2024) and keep the toolchain pinned via `rust-toolchain.toml`.
- Run `cargo fmt` and `cargo clippy -- -D warnings` for touched crates.
- Prefer `axum`/`tower` for HTTP, `sqlx` for Postgres, `serde` for DTOs.

### Code organization conventions

The codebase follows a layered architecture with clear boundaries. See `specs/architecture.md` for full details.

#### Layer separation

1. **Storage Layer** (`everruns-storage`):
   - Database models use `Row` suffix: `AgentRow`, `SessionRow`, `EventRow`
   - Create input structs: `CreateAgentRow`, `CreateEventRow`
   - Update structs: `UpdateAgent`, `UpdateSession`
   - Repositories handle raw database operations only
   - Note: Messages are stored as events (see `specs/models.md`)

2. **Core Layer** (`everruns-core`):
   - Shared domain types: `ContentPart`, `Controls`, `Message`, `ToolCall`
   - Trait definitions: `MessageStore`, `EventEmitter`, `LlmProvider`
   - Types are DB-agnostic and serializable
   - OpenAPI support via feature flag: `#[cfg_attr(feature = "openapi", derive(ToSchema))]`

3. **API Layer** (`everruns-api`):
   - API contracts collocated with routes (e.g., `messages.rs` has routes + DTOs)
   - Services accept API DTOs, transform to storage types, store in database
   - Input types: `InputMessage`, `InputContentPart` (for user-facing input)
   - Request wrappers: `CreateMessageRequest`, `CreateAgentRequest`

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

### Pre-PR checklist

Before creating a pull request, ensure:

1. **Formatting**: Run `cargo fmt` to format all code
2. **Linting**: Run `cargo clippy -- -D warnings` and fix all warnings
3. **Tests**: Run `cargo test` to ensure all tests pass
4. **UI Linting**: Run `npm run lint` in `apps/ui/` to check for oxlint issues
5. **UI Build**: Run `npm run build` in `apps/ui/` to verify TypeScript and build
6. **Docs Build**: Run `npm run check && npm run build` in `apps/docs/` to verify docs build
7. **Smoke tests**: Run smoke tests to verify the system works end-to-end
8. **Examples**: If adding or modifying examples, validate they run successfully against a running API
9. **Update specs**: If your changes affect system behavior, update the relevant specs in `specs/`
10. **Update docs**: If your changes affect usage or configuration, update public docs in `docs/`

```bash
# Quick pre-PR check (Rust)
cargo fmt && cargo clippy -- -D warnings && cargo test

# Quick pre-PR check (UI)
cd apps/ui && npm run lint && npm run build

# Quick pre-PR check (Docs)
cd apps/docs && npm run check && npm run build
```

CI will fail if formatting, linting, tests, UI build, or docs build fail. Always run these locally before pushing.

### UI conventions

- Use **npm** for package management (CI uses `npm ci`)
- After adding dependencies, ensure `package-lock.json` is updated via `npm install`
- Run `npm run lint` to check for oxlint issues
- Run `npm run build` to verify TypeScript types and build before pushing

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

  # Integration tests (requires API running)
  cargo test --test integration_test -- --ignored
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
./scripts/dev.sh init

# Option 1: Start everything at once
./scripts/dev.sh start-all

# Option 2: Start services individually
./scripts/dev.sh start      # Start Docker services
./scripts/dev.sh migrate    # Run migrations
./scripts/dev.sh api        # Start API (in one terminal)
./scripts/dev.sh worker     # Start Temporal worker (in another terminal)
./scripts/dev.sh ui         # Start UI (in another terminal)

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

**Integration tests** (requires API running):
```bash
cargo test --test integration_test -- --ignored
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
│   ├── everruns-api/     # HTTP API (axum), API DTOs
│   ├── everruns-worker/  # Temporal worker
│   ├── everruns-core/    # Core abstractions, domain entities, tools
│   ├── everruns-storage/ # Database layer
│   ├── everruns-openai/  # OpenAI provider
│   └── everruns-anthropic/  # Anthropic provider
├── docs/                 # Documentation content (published via apps/docs)
├── harness/              # Docker Compose
├── specs/                # Specifications
└── scripts/              # Dev scripts
```

