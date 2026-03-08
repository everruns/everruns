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

#### sccache (optional, recommended)

Shared S3 compile cache. Cuts clean builds from ~2m to ~1m. Installed automatically by `init-cloud-env.sh`. To activate in your shell:

```bash
source scripts/lib/sccache.sh && activate_sccache
```

Or via Doppler (wraps credential mapping):

```bash
doppler run -- bash -c 'source scripts/lib/sccache.sh && activate_sccache && cargo build'
```

Check stats: `just sccache-stats`. See `specs/sccache.md` for details.

All cloud secrets are in Doppler (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GITHUB_TOKEN`, `LINEAR_API_KEY`).

### Linear

We use [Linear](https://linear.app) for issue tracking (project: **OSS**, team: **EVE**). MCP server configured in `.mcp.json`. Token (`LINEAR_API_KEY`) is in Doppler. Use [`/process-issues`](.claude/commands/process-issues.md) to batch-process open issues (up to 5 in parallel). All issues for this repo belong to the OSS project.

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

**Spec content principle:** Specs capture *design intent, rationale, and constraints* — the "why" and "what", not exhaustive "how". Don't duplicate what's readable from code (struct fields, enum variants, exact API shapes, SQL DDL). Instead, link to the source file. Example: "See `crates/core/src/models/agent.rs` for full field list." This keeps specs maintainable and prevents drift.

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
- `specs/commands.md` - Slash commands system (system + skill commands)
- `specs/linear-issues.md` - Linear issue processing workflow
- `specs/daytona.md` - Daytona cloud sandbox integration
- `specs/brave-search.md` - Brave Search web search integration
- `specs/duckduckgo.md` - DuckDuckGo instant answer search integration
- `specs/harness-types.md` - Built-in harness types (Base, Generic)
- `specs/client-side-tools.md` - Client-side tools for API/SDK consumers
- `specs/codesandbox.md` - CodeSandbox cloud sandbox integration
- `specs/infinity-context.md` - Unlimited conversation length via context management
- `specs/load-testing.md` - End-to-end load testing framework and benchmarking process
- `specs/apps.md` - Apps system (agent deployment to distribution channels)
- `specs/slack-integration.md` - Slack bot integration (app-scoped webhook, session routing)
- `specs/feature-flags.md` - Feature flags system (env vars, deployment grade, UI gating)
- `specs/tool-search.md` - OpenAI tool_search deferred tool loading capability
- `specs/cache.md` - Caching strategy and distributed rate limiting (Valkey)
- `specs/sccache.md` - Shared compile cache (sccache with S3 backend)

### Skills

`.claude/skills/` contains development skills.

- `no-docker-setup/` - PostgreSQL + Valkey setup for cloud agents
- `ui-screenshots/` - agent-browser UI screenshots

### Test Cases

`test_cases/` - manual test cases by feature. Format in `specs/test-cases.md`.

### Local Dev

```bash
just start-dev          # DEV MODE (in-memory, no Docker)
just start-all          # Full mode (PostgreSQL)
just --list             # All commands
```

#### Worktrees

Use a port prefix per worktree/session. Convention:

- `PORT_PREFIX=xyz`
- proxy/app: `xyz00`
- server: `xyz01`
- frontend: `xyz05`
- postgres: `xyz32` (when needed)

```bash
PORT_PREFIX=271 just start-dev
PORT_PREFIX=271 just start-all
```

- `scripts/lib/services.sh`, `scripts/lib/docker.sh`, `scripts/lib/bench.sh`, and `local/Caddyfile` read `PORT_PREFIX`
- Explicit `API_PORT`, `UI_PORT`, `PROXY_PORT`, and `DB_PORT` still override individual ports if needed
- If `PORT_PREFIX` is unset, repo defaults stay `9000` (API), `9100` (UI), `9300` (proxy), `5432` (Postgres)
- UI-only worktree iteration:

```bash
cd apps/ui
./node_modules/.bin/next dev --port 9120
```

- If `apps/ui/node_modules` is shared into the worktree via symlink, use `--webpack`; Turbopack rejects node_modules outside the worktree root
- If the worktree does not have UI deps yet, install them in `apps/ui` before starting Next

### Rust

- Stable Rust (edition 2024), toolchain in `rust-toolchain.toml`
- `cargo fmt` and `cargo clippy -- -D warnings` for touched crates

### Before Pushing

**Always run `just pre-push` before `git push`.** Fast (~30s) checks that catch most CI failures locally: formatting, clippy, lockfile, UI lint.

If checks fail, auto-fix with `just fmt`, then re-run `just pre-push`.

### Shipping

"Ship" means: implement with extensive test coverage (positive and negative paths), then complete the full Pre-PR Checklist (especially smoke testing impacted functionality in both dev and full modes), create PR, and merge when CI is green.

Use the [`/ship`](.claude/commands/ship.md) command to execute the full shipping workflow. It covers test coverage verification, code simplification, security review, artifact updates (specs, threat model, docs, test cases), smoke testing, quality gates, PR creation, and merge. When asked to "fix and ship", implement the fix first, then run `/ship`.

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
