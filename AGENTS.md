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
- `specs/ag-ui-protocol.md` - AG-UI protocol integration
- `specs/tool-execution.md` - Tool types and execution flow

Specification format: Abstract and Requirements sections.

### Skills

`.claude/skills/` contains development skills following the [Agent Skills Specification](https://github.com/anthropics/skills/blob/main/spec/agent-skills-spec.md).

Available skills:
- `smoke-tests/` - API and UI smoke testing with support for Docker and no-Docker environments

### Local dev expectations

- A `harness/docker-compose.yml` brings up Temporal + Postgres + required dependencies
- `protoc` (Protocol Buffers compiler) is required for building Temporal SDK dependencies
  - Debian/Ubuntu: `apt-get install protobuf-compiler`
  - macOS: `brew install protobuf`
  - Or download from https://github.com/protocolbuffers/protobuf/releases

### Cloud Agent environments

When running in cloud-hosted agent environments (e.g., Claude Code on the web), the following secrets are available:

- `OPENAI_API_KEY`: Available for LLM-related operations
- `GITHUB_TOKEN`: Available for GitHub API operations (PRs, issues, repository access)

These secrets are pre-configured in the environment and do not require manual setup.

### Rust conventions

- Use stable Rust (edition 2024) and keep the toolchain pinned via `rust-toolchain.toml`.
- Run `cargo fmt` and `cargo clippy -- -D warnings` for touched crates.
- Prefer `axum`/`tower` for HTTP, `sqlx` for Postgres, `serde` for DTOs.

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
4. **UI Linting**: Run `npm run lint` in `apps/ui/` to check for ESLint issues
5. **UI Build**: Run `npm run build` in `apps/ui/` to verify TypeScript and build
6. **Smoke tests**: Run smoke tests to verify the system works end-to-end
7. **Update specs**: If your changes affect system behavior, update the relevant specs in `specs/`
8. **Update docs**: If your changes affect usage or configuration, update public docs in `./docs` folder

```bash
# Quick pre-PR check (Rust)
cargo fmt && cargo clippy -- -D warnings && cargo test

# Quick pre-PR check (UI)
cd apps/ui && npm run lint && npm run build
```

CI will fail if formatting, linting, tests, or UI build fail. Always run these locally before pushing.

### UI conventions

- Use **npm** for package management (CI uses `npm ci`)
- After adding dependencies, ensure `package-lock.json` is updated via `npm install`
- Run `npm run lint` to check for ESLint issues
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

### Pull request conventions

PR titles should follow Conventional Commits format. Use the PR template (`.github/pull_request_template.md`) for descriptions.

**PR Description Template:**

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

