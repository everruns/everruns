## Coding-agent guidance

### Cloud Agent (start here)

Use Doppler for all secret-backed commands in cloud agents.

```bash
./scripts/init-cloud-env.sh
doppler run -- just start-dev --no-watch
```

All cloud secrets are in Doppler (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GITHUB_TOKEN`).

For GitHub CLI, map token explicitly:

```bash
doppler run -- bash -lc 'GH_TOKEN="$GITHUB_TOKEN" gh auth status'
```

Quickcheck:

```bash
doppler run -- env | rg 'OPENAI_API_KEY|ANTHROPIC_API_KEY|GITHUB_TOKEN'
doppler run -- bash -lc 'GH_TOKEN="$GITHUB_TOKEN" gh auth status'
```

### Style

Telegraph. Drop filler/grammar. Min tokens.

### Critical Thinking

Fix root cause. Unsure: read more code; if stuck, ask w/ short options. Unrecognized changes: assume other agent; keep going. If causes issues, stop + ask.

### Principles

- Important decisions as comments on top of file
- Code testable, smoke testable, runnable locally
- Small, incremental PR-sized changes
- No backward compat needed (internal code)
- Write failing test before fixing bug

### Specs

`specs/` contains feature specifications. New code should comply with these or propose changes.

- `specs/concepts.md` - Core entities, relationships, and concept diagram
- `specs/architecture.md` - System architecture, crate structure, infrastructure
- `specs/code-organization.md` - Naming conventions, type flow, testing, error handling
- `specs/models.md` - Data models (Agent, Session, Message, etc.)
- `specs/apis.md` - HTTP API endpoints, error handling
- `specs/events.md` - Event types and SSE streaming
- `specs/markdown-messages.md` - Chat message markdown rendering with llm-ui
- `specs/tool-execution.md` - Tool types and execution flow
- `specs/capabilities.md` - Agent capabilities system
- `specs/mcp-servers.md` - MCP server registration
- `specs/llm-drivers.md` - LLM driver trait, provider implementations
- `specs/durable-execution-engine.md` - PostgreSQL-backed durable workflow engine
- `specs/scheduled-tasks.md` - Cron-based scheduled tasks for durable engine
- `specs/fail-rs-testing.md` - Failure injection testing with fail-rs
- `specs/authentication.md` - Authentication modes and OAuth
- `specs/encryption.md` - Envelope encryption for sensitive data
- `specs/session-filesystem.md` - Per-session virtual filesystem
- `specs/usage-tracking.md` - LLM token usage tracking
- `specs/documentation.md` - Documentation site (Astro Starlight)
- `specs/brand.md` - Brand identity, colors, typography
- `specs/dismissed-options.md` - Technical options considered but dismissed
- `specs/multitenancy.md` - Organization-based multitenancy
- `specs/release-process.md` - Release workflow with CHANGELOG.md
- `specs/id-schema.md` - Standardized prefixed ID format
- `specs/braintrust-integration.md` - Braintrust observability
- `specs/otel-observability.md` - OpenTelemetry Gen-AI semantic convention tracing
- `specs/test-cases.md` - Manual test case format
- `specs/session-sqldb.md` - Session-scoped SQL databases (SQLite over PostgreSQL VFS)
- `specs/threat-model.md` - Security threat model with stable IDs and mitigations
- `specs/bashkit-requirements.md` - Bash sandbox capabilities and requirements
- `specs/events-contract.md` - SSE event format contract
- `specs/maintenance.md` - Pre-release maintenance checklist

### Skills

`.claude/skills/` contains development skills.

- `smoke-test/` - API and UI smoke testing
- `no-docker-setup/` - PostgreSQL setup for cloud agents
- `ui-screenshots/` - agent-browser screenshots for PR comments

### Test Cases

`test_cases/` - manual test cases by feature. Format in `specs/test-cases.md`.

### Local Dev

```bash
just start-dev          # DEV MODE (in-memory, no Docker)
just start-all          # Full mode (PostgreSQL)
just --list             # All commands
```

### Rust

- Stable Rust (edition 2024), toolchain in `rust-toolchain.toml`
- `cargo fmt` and `cargo clippy -- -D warnings` for touched crates

### Pre-PR Checklist

1. `just pre-pr` (runs 2-7 automatically)
2. `cargo fmt --check`
3. `cargo clippy --all-targets --all-features -- -D warnings`
4. `cargo test --all-features`
5. `npm run lint` + `npm run build` in `apps/ui/`
6. OpenAPI spec fresh: `./scripts/export-openapi.sh`
7. Docs build: `npm run build` in `apps/docs/`
8. Rebase on main: `git fetch origin main && git rebase origin/main`
9. Smoke test new functionality
10. UI screenshots for UI changes (use `.claude/skills/ui-screenshots/`)
11. Test coverage: tests must reproduce issue + verify fix, cover touched code paths
12. CI green before merge
13. Resolve all PR comments

### CI

- GitHub Actions. Check via `gh` tool.
- **NEVER merge when CI is red.** No exceptions.

### Commits

[Conventional Commits](https://www.conventionalcommits.org): `type(scope): description`

Types: feat, fix, docs, refactor, test, chore

Use `chore` for updates to `specs/` and `AGENTS.md`.

### PRs

**REQUIRED:** Use `.github/pull_request_template.md`. Squash and Merge.

**NEVER** add links to Claude sessions in PR body or commits.

See `CONTRIBUTING.md` for details.
