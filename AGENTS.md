## Coding-agent guidance

### Cloud Agent (start here)

Use Doppler for all secret-backed commands in cloud agents.

```bash
./scripts/init-cloud-env.sh
doppler run -- just start-dev --no-watch
```

Disable incremental compilation in cloud (saves ~3 GB, useless for single builds):

```bash
export CARGO_INCREMENTAL=0
```

All cloud secrets are in Doppler (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GITHUB_TOKEN`, `LINEAR_API_KEY`).

### Linear

We use [Linear](https://linear.app) for issue tracking. MCP server configured in `.mcp.json`. Token (`LINEAR_API_KEY`) is in Doppler.

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
- `specs/agent-instructions.md` - AGENTS.md support (dynamic project instructions)
- `specs/mcp-servers.md` - MCP server registration
- `specs/llm-drivers.md` - LLM driver trait, provider implementations
- `specs/durable-execution-engine.md` - PostgreSQL-backed durable workflow engine
- `specs/scheduled-tasks.md` - Cron-based scheduled tasks for durable engine
- `specs/fail-rs-testing.md` - Failure injection testing with fail-rs
- `specs/authentication.md` - Authentication modes and OAuth
- `specs/user-connections.md` - User connections (GitHub, GitLab) for repo access
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
- `specs/xml-prompt-formatting.md` - XML tags for system prompt structure
- `specs/skills-registry.md` - Agent Skills registry (agentskills.io format)
- `specs/linear-issues.md` - Linear issue processing workflow
- `specs/daytona.md` - Daytona cloud sandbox integration
- `specs/brave-search.md` - Brave Search web search integration
- `specs/harness-types.md` - Built-in harness types (Base, Generic)
- `specs/client-side-tools.md` - Client-side tools for API/SDK consumers
- `specs/codesandbox.md` - CodeSandbox cloud sandbox integration
- `specs/infinity-context.md` - Unlimited conversation length via context management
- `specs/load-testing.md` - End-to-end load testing framework and benchmarking process

### Skills

`.claude/skills/` contains development skills.

- `smoke-test/` - API and UI smoke testing
- `no-docker-setup/` - PostgreSQL setup for cloud agents
- `ui-screenshots/` - agent-browser UI screenshots

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

### Before Pushing

**Always run `just pre-push` before `git push`.** Fast (~30s) checks that catch most CI failures locally: formatting, clippy, lockfile, UI lint.

If checks fail, auto-fix with `just fmt`, then re-run `just pre-push`.

### Shipping

"Ship" means: implement with extensive test coverage (positive and negative paths), then complete the full Pre-PR Checklist (especially smoke testing impacted functionality in both dev and full modes), create PR, and merge when CI is green.

### Pre-PR Checklist

1. `just pre-push` (fast: fmt, lint, lockfile)
2. `just pre-pr` (full: runs 3-8 automatically)
3. `cargo fmt --check`
4. `cargo clippy --all-targets --all-features -- -D warnings`
5. `cargo test --all-features`
6. `npm run lint` + `npm run build` in `apps/ui/`
7. OpenAPI spec fresh: `./scripts/export-openapi.sh`
8. Docs build: `npm run build` in `apps/docs/`
9. Rebase on main: `git fetch origin main && git rebase origin/main`
10. Smoke test impacted functionality in both dev mode (`just start-dev`) and full mode (`just start-all`)
11. Performance impact: no unindexed queries, no full table scans, no N+1 queries, no unbounded result sets; add pagination/limits where needed
12. UI screenshots for UI changes
13. Test coverage: extensive positive and negative tests; reproduce issue + verify fix, cover touched code paths
14. Update relevant specs in `specs/`
15. Update docs in `apps/docs/` if applicable
16. CI green before merge
17. Resolve all PR comments

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
