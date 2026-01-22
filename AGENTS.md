## Coding-agent guidance

### Style

Telegraph. Drop filler/grammar. Min tokens.

### Critical Thinking

Fix root cause. Unsure: read more code; if stuck, ask w/ short options. Unrecognized changes: assume other agent; keep going. If causes issues, stop + ask.

### Principles

- Important decisions as comments on top of file
- Code testable, smoke testable, runnable locally
- Small, incremental PR-sized changes
- No backward compat needed (internal code)

### Specs

`specs/` contains feature specifications. New code should comply with these or propose changes.

- `specs/architecture.md` - System architecture, crate structure, infrastructure
- `specs/code-organization.md` - Naming conventions, type flow, testing, error handling
- `specs/models.md` - Data models (Agent, Session, Message, etc.)
- `specs/apis.md` - HTTP API endpoints, error handling
- `specs/events.md` - Event types and SSE streaming
- `specs/tool-execution.md` - Tool types and execution flow
- `specs/capabilities.md` - Agent capabilities system
- `specs/mcp-servers.md` - MCP server registration
- `specs/llm-drivers.md` - LLM driver trait, provider implementations
- `specs/durable-execution-engine.md` - PostgreSQL-backed durable workflow engine
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
- `specs/test-cases.md` - Manual test case format

### Skills

`.claude/skills/` contains development skills.

- `smoke-test/` - API and UI smoke testing
- `no-docker-setup/` - PostgreSQL setup for cloud agents
- `ui-screenshots/` - Playwright screenshots for PR comments

### Test Cases

`test_cases/` - manual test cases by feature. Format in `specs/test-cases.md`.

### Cloud Agent Start

```bash
./scripts/init-cloud-env.sh       # Install just + gh
just start-dev --no-watch         # DEV MODE (no Docker)
```

Pre-configured: `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GITHUB_TOKEN`

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
11. CI green before merge
12. Resolve all PR comments

### CI

- GitHub Actions. Check via `gh` tool.
- **NEVER merge when CI is red.** No exceptions.

### Commits

[Conventional Commits](https://www.conventionalcommits.org): `type(scope): description`

Types: feat, fix, docs, refactor, test, chore

### PRs

**REQUIRED:** Use `.github/pull_request_template.md`. Squash and Merge.

See `CONTRIBUTING.md` for details.
