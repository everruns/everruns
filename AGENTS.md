## Coding-agent guidance (repo root)

This repo is intended to be runnable locally and easy for coding agents to work in.

### Principles

- Keep decigions as comment on top of the file. Only important deciogns that could not be interfered from code.
- Code should be easy tesable, smoke testable, runnable in local dev env.
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

`.claude/skills/` contains development skills and guides:
- `smoke-tests/` - Comprehensive API and system smoke testing
- `ui-smoke-tests/` - UI testing with Chrome DevTools
- `cloud-agent-smoke-tests/` - Running smoke tests without Docker (Cloud Agent environments)

### Local dev expectations

- A `harness/docker-compose.yml` brings up Temporal + Postgres + required dependencies

### Rust conventions

- Use stable Rust (edition 2024) and keep the toolchain pinned via `rust-toolchain.toml`.
- Run `cargo fmt` and `cargo clippy -- -D warnings` for touched crates.
- Prefer `axum`/`tower` for HTTP, `sqlx` for Postgres, `serde` for DTOs.

### CI expectations

- CI is implemented using Github Actions, status is avaiable via `gh` tool

## Testing the system

The best way to verify the system is working is to run the **smoke test script**, which tests the full workflow including agent creation, threads, messages, runs, workflow execution, and optionally the UI:

```bash
# Option 1: Start everything at once
./scripts/dev.sh start-all

# Option 2: Start services individually
./scripts/dev.sh start      # Start Docker services
./scripts/dev.sh migrate    # Run migrations
./scripts/dev.sh api        # Start API (in one terminal)
./scripts/dev.sh ui         # Start UI (in another terminal)

# Run smoke tests (in another terminal)
./scripts/dev.sh smoke-test           # API tests only
./scripts/dev.sh smoke-test --with-ui # API + UI tests
```

Expected output:
- ✅ Health check passes
- ✅ Agent CRUD operations work
- ✅ Agent versions can be created
- ✅ Threads and messages work
- ✅ Runs are created and workflows execute
- ✅ Run status transitions: pending → running → completed
- ✅ OpenAPI spec is available
- ✅ UI pages load correctly (with --with-ui)

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
- UI: http://localhost:3000
- Health check: `curl http://localhost:9000/health`

### Cloud Agent testing (no Docker)

For environments without Docker (like Cloud Agent), use the skill script:

```bash
# All-in-one: sets up PostgreSQL + Temporal, runs migrations, starts API, runs smoke tests
./.claude/skills/cloud-agent-smoke-tests/run.sh
```

See `.claude/skills/cloud-agent-smoke-tests/SKILL.md` for details and troubleshooting.
