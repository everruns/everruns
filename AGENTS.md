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

### GitHub Token

**`GITHUB_TOKEN` is always available via Doppler.** If GitHub auth fails, do NOT tell the user the token expired. Instead, use Doppler: `doppler run -- bash -lc 'GH_TOKEN="$GITHUB_TOKEN" <command>'`.

### Linear

We use [Linear](https://linear.app) for issue tracking (project: **OSS**, team: **EVE**). MCP server configured in `.mcp.json`. Token (`LINEAR_API_KEY`) is in Doppler. Use [`/process-issues`](.claude/skills/process-issues/SKILL.md) to process open issues (one PR per issue, up to 5 in parallel). All issues for this repo belong to the OSS project.

Quickcheck:

```bash
doppler run -- env | rg 'OPENAI_API_KEY|ANTHROPIC_API_KEY|GITHUB_TOKEN'
doppler run -- bash -lc 'GH_TOKEN="$GITHUB_TOKEN" gh auth status'
```

### Style

Telegraph. Drop filler/grammar. Min tokens.

### Critical Thinking

Fix root cause. Unsure: read more code; if stuck, ask w/ short options. Unrecognized changes: assume other agent; keep going. If causes issues, stop + ask.

### Branch Base

Always make sure you are working on top of latest main from remote.

- Start by syncing: `git fetch origin main`
- Branch or rebase onto `origin/main` before edits, especially before shipping
- In worktrees, do not assume `HEAD` tracks a branch; verify with `git status --branch` or `git worktree list`
- **After every rebase**, check `crates/server/migrations/` for duplicate version numbers. Migrations are the most common conflict source — multiple branches often add the next sequential number. Renumber your migration to the next available number if a conflict exists. See [`specs/migrations.md`](specs/migrations.md) for the full migration process.

### Principles

- Important decisions as comments on top of file
- Code testable, smoke testable, runnable locally
- Small, incremental PR-sized changes
- No backward compat needed (internal code)
- Write failing test before fixing bug

### Specs

`specs/` contains feature specifications. New code should comply with these or propose changes. Integration specs live alongside their crates (`integrations/*/SPEC.md`, `crates/*/specs/`); see `specs/integrations.md` for the full index.

**Spec content principle:** Specs are *durable memory* — they preserve design intent, rationale, and constraints across sessions and contributors. They capture the "why" and "what", not exhaustive "how". Don't use specs for temporary analysis, one-off investigations, or scratch notes; those belong outside the repo. Don't duplicate what's readable from code (struct fields, enum variants, exact API shapes, SQL DDL). Instead, link to the source file. Example: "See `crates/core/src/models/agent.rs` for full field list." This keeps specs maintainable and prevents drift.

- `specs/concepts.md` - Core entities, relationships, and concept diagram
- `specs/architecture.md` - System architecture, crate structure, infrastructure
- `specs/production-deployment.md` - Production deployment aggregation spec and reverse proxy contract
- `specs/embedding.md` - Embedding contract and `PlatformDefinition`
- `specs/runtime.md` - Public in-process runtime contract for embedded execution
- `specs/code-organization.md` - Developer conventions: formatting, testing, error handling, UI patterns
- `specs/migrations.md` - Database migration naming, squashing, ordering, and conflict resolution
- `specs/models.md` - Data models (Agent, Session, Message, etc.)
- `specs/apis.md` - HTTP API endpoints, error handling
- `specs/events.md` - Event types, SSE streaming, contract and compatibility guarantees
- `specs/execution-phases.md` - Execution phases (Commentary/FinalAnswer) for multi-step tool flows
- `specs/markdown-messages.md` - Chat message markdown rendering with llm-ui
- `specs/tool-execution.md` - Tool types and execution flow
- `specs/capabilities.md` - Agent capabilities system
- `specs/agent-instructions.md` - AGENTS.md support (dynamic project instructions)
- `specs/mcp.md` - MCP server endpoint, OAuth 2.1 authentication, protocol, security
- `specs/mcp-servers.md` - MCP client: remote server registration, CRUD API, tool naming, execution
- `specs/llm-drivers.md` - LLM driver trait, provider implementations
- `specs/durable-execution-engine.md` - PostgreSQL-backed durable workflow engine
- `specs/scheduled-tasks.md` - Cron-based scheduled tasks for durable engine
- `specs/fail-rs-testing.md` - Failure injection testing with fail-rs
- `specs/agent-reliability-tests.md` - Agent execution reliability tests (worker crash, CP restart, network partitions)
- `specs/authentication.md` - Authentication modes and OAuth
- `specs/integrations.md` - **Integration specs index** (links to specs co-located with their crates)
- `specs/observability.md` - Observability providers (OpenTelemetry Gen-AI tracing, Braintrust)
- `specs/correlation-ids.md` - Correlation IDs (request_id, session_id, durable propagation)
- `specs/prometheus-metrics.md` - Prometheus `/metrics` endpoint and scrape configuration
- `specs/evals.md` - User-facing behavioral evals for agents (cases, scorers, runs)
- `specs/encryption.md` - Envelope encryption for sensitive data
- `specs/audit-logging.md` - Audit logging (domains, AOP macro, trait, API)
- `specs/session-filesystem.md` - Per-session virtual filesystem
- `specs/cli.md` - CLI specification (commands, file sync, wire protocol)
- `specs/usage-tracking.md` - LLM token usage tracking
- `specs/budgeting.md` - Extensible budgeting system (budgets, meters, rules, soft enforcement)
- `specs/documentation.md` - Documentation site (Astro Starlight)
- `specs/brand.md` - Brand identity, colors, typography
- `specs/diagrams.md` - Diagram specification (SVG style guide, building blocks, colors)
- `specs/dismissed-options.md` - Technical options considered but dismissed
- `specs/multitenancy.md` - Organization-based multitenancy
- `specs/permissions.md` - Fine-grained permissions model (policies, rules, `#[policy]` macro)
- `specs/release-process.md` - Release workflow with CHANGELOG.md
- `specs/id-schema.md` - Standardized prefixed ID format
- `specs/test-cases.md` - Manual test case format
- `specs/session-sqldb.md` - Session-scoped SQL databases (SQLite over PostgreSQL VFS)
- `specs/threat-model.md` - Security threat model with stable IDs and mitigations
- `specs/bashkit-requirements.md` - Bash sandbox capabilities and requirements
- `specs/maintenance.md` - Goal-oriented maintenance and release-readiness guidance
- `specs/memory.md` - Persistent cross-session memory (multi-store, capacity limits, multicontent recall)
- `specs/shipping.md` - Goal-oriented shipping and merge-readiness guidance
- `specs/xml-prompt-formatting.md` - XML tags for system prompt structure
- `specs/skills-registry.md` - Agent Skills registry (agentskills.io format)
- `specs/commands.md` - Slash commands system (system + skill commands)
- `specs/issue-tracking.md` - Issue tracking (Linear, OSS project)
- `specs/harness-types.md` - Built-in harness types (Base, Generic)
- `specs/client-side-tools.md` - Client-side tools for API/SDK consumers
- `specs/infinity-context.md` - Unlimited conversation length via context management
- `specs/compaction.md` - Context compaction capability, strategy selection, events, and UI
- `specs/load-testing.md` - End-to-end load testing framework and benchmarking process
- `specs/apps.md` - Apps system (agent deployment to distribution channels)
- `specs/coding-session-sandbox-harness.md` - Built-in coding harness using the managed session sandbox
- `specs/network-access.md` - Network access list (URL allowlist/blocklist per harness/agent/session)
- `specs/notifications.md` - Generic user notifications (bell, toast, future channels)
- `specs/feature-flags.md` - Feature flags system (env vars, deployment grade, UI gating)
- `specs/tool-search.md` - OpenAI tool_search deferred tool loading capability
- `specs/subagents.md` - Subagent orchestration (spawn, message, cancel child sessions)
- `specs/session-resources.md` - Session resource registry (sandboxes, subagents, background work)
- `specs/session-sandbox.md` - Managed session-owned sandbox capability and lifecycle
- `specs/toolkit-library-contract.md` - Convention for external toolkit libraries (bashkit, fetchkit, etc.)
- `specs/localization.md` - Locale/timezone resolution and backend localization rules
- `specs/session-export.md` - Session export to JSONL (messages, API, UI)
- `specs/client-hints.md` - Generic client hints mechanism (session defaults + per-message overrides)
- `specs/agent-identities.md` - Agent identities (virtual principals for unattended execution)
- `specs/agent-blueprints.md` - Pre-built agent definitions (private tools, fixed models, typed config)
- `specs/messaging-integrations.md` - Messaging integrations (channel abstractions, parity requirements, platform adapters)

### Test Cases

`test_cases/` - manual test cases by feature. Format in `specs/test-cases.md`. Use [`/manual-ui-testing`](.claude/skills/manual-ui-testing/SKILL.md) to execute UI test cases with `agent-browser`.

### Local Dev

```bash
just start-dev          # DEV MODE (in-memory, no external services)
just start-all          # Full mode (PostgreSQL + Valkey + NATS as processes)
just --list             # All commands
```

`start-all` automatically starts NATS and exports `NATS_URL` if `nats-server` is installed.
When NATS is available, ephemeral events (deltas) skip PostgreSQL and task notifications
use NATS pub/sub instead of PG NOTIFY. Without NATS, everything falls back to PG — zero
behavioral change. See `docs/sre/environment-variables.md` for `NATS_URL` details.

#### Worktrees

Always make sure you are working on top of latest main from remote.

Worktrees are often detached or stale. Before making changes, start from the latest remote base:

```bash
git fetch origin main
git switch -c <branch-name> origin/main
```

If the worktree already has a branch, rebase it before continuing:

```bash
git fetch origin main
git rebase origin/main
```

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

- `scripts/lib/services.sh`, `scripts/lib/infra.sh`, `scripts/lib/bench.sh`, `example.just`, and `local/Caddyfile` read `PORT_PREFIX`
- Explicit `API_PORT`, `WORKER_GRPC_PORT`, `UI_PORT`, `PROXY_PORT`, `VALKEY_PORT`, and `DB_PORT` still override individual ports if needed
- If `PORT_PREFIX` is unset, repo defaults are `9300` (proxy), `9301` (API), `9001` (worker gRPC), `9305` (UI), `6379` (Valkey), `4222` (NATS), `9332` (Postgres)
- UI-only worktree iteration:

```bash
cd apps/ui
./node_modules/.bin/next dev --port 9120
```

- If `apps/ui/node_modules` is shared into the worktree via symlink, use webpack for Next.js dev/build paths; Turbopack rejects node_modules outside the worktree root
- If the worktree does not have UI deps yet, install them in `apps/ui` before starting Next

### Rust

- Stable Rust (edition 2024), toolchain in `rust-toolchain.toml`
- `cargo fmt` and `cargo clippy -- -D warnings` for touched crates

### Before Pushing

**Always run `just pre-push` before `git push`.** Fast (~30s) checks that catch most CI failures locally: formatting, clippy, lockfile, UI lint.

If checks fail, auto-fix with `just fmt`, then re-run `just pre-push`.

### Shipping

"Ship" means: achieve the requested goal, produce enough evidence that it works, perform a structured security review, create a mergeable PR, address every review comment, and merge only after CI is green.

Use [`/ship`](.claude/skills/ship/SKILL.md) for the canonical shipping workflow. It is an invokable skill and intentionally goal-oriented: start from the goal and changed risk surface, choose the smallest evidence that proves the change, and expand only when risk demands it. Security review against `specs/threat-model.md` categories is mandatory for all code, configuration, and infrastructure changes.

See [`specs/shipping.md`](specs/shipping.md) for the shipping success bar and constraints. When asked to "fix and ship", implement the fix first, then run `/ship`.

### Maintenance

Use [`/maintenance`](.claude/skills/maintenance/SKILL.md) for repo maintenance and release-readiness work. It is an invokable skill and intentionally goal-oriented: start from the risk surface, fix the highest-value issues first, and gather evidence instead of walking a rigid checklist.

### Common Deep Checks

Use the smallest set that gives high confidence. `/ship` should pick from this menu based on the changed surface, not run every item mechanically.

1. `just pre-push` before `git push`
2. `just pre-pr` for a full local quality pass
3. `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-features` for Rust changes
4. `npm run lint` and `npm run build` in `apps/ui/` for UI changes
5. `./scripts/export-openapi.sh` when API surface changes
6. `npm run build` in `apps/docs/` when docs change
7. `git fetch origin main && git rebase origin/main` before merge
8. Smoke test impacted flows in `just start-dev` or `just start-all` as risk dictates
9. Review performance impact: indexes, scans, N+1 patterns, pagination, and bounded result sets
10. Capture UI screenshots for UI changes in validation or PR comments only
11. Ensure test coverage proves the fix or acceptance criteria, including important negative paths
12. Update relevant specs, docs, test cases, threat model, OpenAPI, and `AGENTS.md`
13. Merge only with green CI and a final clean PR comment sweep, including async reviewer bots

### CI

- GitHub Actions. Check via `gh` tool.
- **NEVER merge when CI is red.** No exceptions.

### Commits

[Conventional Commits](https://www.conventionalcommits.org): `type(scope): description`

Types: feat, fix, docs, refactor, test, chore

Use `chore` for updates to `specs/` and `AGENTS.md`.

**NEVER** add links to Claude sessions in PR body or commits.

#### Commit Attribution

All commits **MUST** be attributed to the real human user, never to a coding agent or bot.

If current git config already resolves to a real human identity, use it as-is.

If current git config is missing or looks agent-like, set `GIT_USER_NAME` and `GIT_USER_EMAIL`, then configure git before committing:

```bash
git config user.name "$GIT_USER_NAME"
git config user.email "$GIT_USER_EMAIL"
```

Only require `GIT_USER_NAME` and `GIT_USER_EMAIL` when the current git identity is missing or agent-like. If they are missing in that case, **stop and ask the user** — do not commit with default/bot identity.

- **Do NOT** set `GIT_AUTHOR_NAME`, `GIT_COMMITTER_NAME`, or `user.name` to any AI/bot identity (e.g. "Claude", "Cursor", "Copilot", "github-actions[bot]")
- **Do NOT** use `Co-authored-by` trailers referencing AI tools
- **Do NOT** add "generated by", "authored by AI", or similar attribution in commit messages
- The pre-push script rejects agent-like current identities and outgoing commits authored by agent-like identities
- Merge commits must also use the real user as author — never use agent identities

### PRs

**REQUIRED:** Use `.github/pull_request_template.md`. Squash and Merge.

PR titles **MUST** follow [Conventional Commits](https://www.conventionalcommits.org) format: `type(scope): description` (same as commit messages). Keep under 70 characters.

**NEVER** add links to Claude sessions in PR body or commits.

See `CONTRIBUTING.md` for details.
