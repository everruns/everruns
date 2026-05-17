---
name: maintenance
description: Goal-oriented repository maintenance and release-readiness work. Use when the user asks for maintenance, release prep, repo health review, dependency refreshes, spec/docs alignment, test gap review, technical debt analysis, or general cleanup without prescribing an exact sequence.
user-invocable: true
---

# Maintenance

Goal: leave the repo materially healthier and closer to release-ready state, with evidence.

This skill implements [`specs/maintenance.md`](../../../specs/maintenance.md). Keep operational guidance here. Keep design intent and constraints in the spec.

This skill is outcome-oriented. Do not blindly walk a fixed checklist. Choose the smallest set of actions that closes the real maintenance risk in front of you.

## When To Use

Use this skill when the task is about repo maintenance rather than a single feature:

- release-readiness review
- dependency refreshes
- spec or docs drift
- test coverage gaps
- threat-model or security hygiene review
- performance review of recently changed code
- technical debt analysis and issue tracking
- AGENTS/skills/command hygiene

## Required Outcomes

1. The maintenance scope is explicit.
   - If the user provided a scope, use it.
   - If not, infer a reasonable scope from recent changes, release posture, and obviously stale areas. State the assumption.
2. The work produces concrete improvement.
   - Fix issues when the change is small and local.
   - If an issue is too large for the current task, capture a crisp finding with evidence and the next action.
3. Validation matches risk.
   - Run checks that prove the updated areas are healthy.
   - Increase depth for auth, persistence, migrations, public API, external integrations, and end-to-end UI flows.
4. A release claim is backed by evidence.
   - Do not call the repo release-ready unless the changed or high-risk surfaces were actually checked.

## Operating Model

- Start from goals and risk surface, not checklist order.
- Prefer the highest-signal path first: recent diffs, flaky areas, failing checks, stale specs, outdated dependencies, or known security/performance hotspots.
- Always run `cargo outdated` (or `cargo search` per-crate) and `npm outdated` during release-readiness or dependency-scoped maintenance — even when no security advisory exists. Patch/minor bumps are cheap to miss and cheap to apply; skipping them silently accumulates drift.
- Check Linear issues in the OSS project (EVE team) already in `In Progress` when maintenance covers release readiness or workflow hygiene. Treat issues whose `updatedAt` is older than 1 day as stale by default, then triage or report them.
- When maintenance covers release readiness or repo workflow hygiene, review recent upstream plugin-platform changes before declaring local plugins current. Check the Codex and Claude Code Everruns Dev plugin surfaces together: compare `.agents/plugins/marketplace.json`, `.claude-plugin/marketplace.json`, `plugins/everruns-dev/.codex-plugin/plugin.json`, `plugins/everruns-dev/.claude-plugin/plugin.json`, shipped plugin behavior, skills, docs, and marketplace entries; run `scripts/test-everruns-dev-plugin.sh` or equivalent metadata validation so registration, version parity, compatibility, and non-contradiction are proven.
- When maintenance covers recently shipped features or release readiness, check for half-built cross-surface features: UI disconnected from backend behavior, backend capabilities missing intended MCP/CLI/docs exposure, MCP or CLI behavior lagging API semantics, or tests/manual cases claiming more than the product provides.
- Skip untouched areas when there is a clear reason. Say why they were skipped.
- Prefer fixing over reporting.
- For bugs uncovered during maintenance, prefer a failing test before the fix when practical.
- Keep changes PR-sized. If a maintenance theme explodes in scope, finish the highest-value slice and report the boundary.

## Maintenance Surfaces

Use judgment on which surfaces matter for the current task.

### Dependency Health

Goal: all packages — including CLI, server, worker, integrations, UI, and docs — run on current dependency versions. Outdated major versions are upgraded proactively, not deferred indefinitely.

Actions:
- audit every workspace crate and npm package for outdated dependencies, including major-version bumps
- upgrade major versions when the migration path is clear; document blockers when it is not
- flag deprecated crates/packages and identify replacements
- check for unused dependencies (`cargo udeps` or manual review)

Good evidence:
- `cargo outdated` (or `cargo search`) checked for each CLI and workspace dependency
- `npm outdated` checked for `apps/ui/` and `apps/docs/`
- major-version upgrades applied and tested, not just noted
- deprecated dependencies flagged with replacement plan
- lockfiles updated intentionally
- relevant build/lint/test checks pass

### Specs And Docs Alignment

Goal: docs describe the current system intent and constraints, without drifting into code duplication.

Good evidence:
- changed behavior reflected in `specs/`, `apps/docs/`, OpenAPI, or examples when relevant
- stale or duplicate spec detail removed in favor of links to source files

### Feature Completeness Across Surfaces

Goal: features that appear shipped in one surface are connected, reachable, and consistent across the intended UI, backend, MCP, CLI, docs, and tests.

Good evidence:
- UI affordances call real backend APIs and handle loading, errors, auth, and state refresh
- backend features intended for agent or automation use are exposed through MCP/app surfaces where applicable
- CLI commands and flags match current API semantics and do not duplicate stale assumptions
- docs, specs, examples, tests, and manual test cases do not claim unavailable behavior
- gaps are fixed locally or captured as specific findings with the missing surface, user impact, and next action

### Security And Threat Posture

Goal: new or changed attack surface is understood, and mitigations/docs match reality.

Actions:
- run DeepSec when maintenance includes security posture, release readiness, auth/tenant/public-ingress review, or repo-wide hygiene:
  - if `.deepsec/` is missing, initialize it with `npx deepsec init`
  - install from `.deepsec/` with `pnpm install`
  - keep `data/<project>/INFO.md` short and project-specific before processing
  - run `pnpm deepsec scan --project-id everruns` from `.deepsec/`
  - use `pnpm deepsec process --project-id everruns --agent codex` only with an explicit budgeted focus (`--filter`, `--limit`, `--only-slugs`, or `--manifest`) unless the user asks for a full AI pass
  - revalidate high-severity results with `pnpm deepsec revalidate --project-id everruns --agent codex --min-severity HIGH`
- keep durable DeepSec workspace files tracked: `.deepsec/.gitignore`, `.deepsec/AGENTS.md`, `.deepsec/README.md`, `.deepsec/deepsec.config.ts`, `.deepsec/package.json`, `.deepsec/pnpm-lock.yaml`, `.deepsec/pnpm-workspace.yaml`, and `.deepsec/data/*/{INFO.md,SETUP.md}`
- do not commit generated DeepSec state unless explicitly requested: `.deepsec/node_modules/`, `.deepsec/.env*.local`, `.deepsec/data/*/{files,runs,reports,project.json,tech.json}`
- create Linear issues in the OSS project (EVE team) for actionable DeepSec findings that are not fixed in the current maintenance pass

Good evidence:
- threat model updated when behavior or trust boundaries changed
- obvious gaps in auth, validation, secret handling, or data exposure were reviewed
- DeepSec scan/process/revalidate run IDs, scope, finding count, and budget/cost noted when DeepSec was used
- [GitHub Security Overview](https://github.com/everruns/everruns/security) checked for advisories
- [Dependabot alerts](https://github.com/everruns/everruns/security/dependabot) reviewed and triaged
- [Secret scanning alerts](https://github.com/everruns/everruns/security/secret-scanning?query=is%3Aopen+results%3Ageneric) reviewed — no open generic secret leaks

### Test And Runtime Confidence

Goal: important paths are covered by the right proof, not ceremony.

Good evidence:
- targeted tests added or updated for regressions
- smoke tests or manual verification used where unit tests are insufficient
- checks match the touched surface instead of running an arbitrary full matrix

### Performance And Operational Safety

Goal: recent changes do not introduce obvious scale or latency regressions.

Good evidence:
- query shape, pagination, indexes, batching, and background job cost reviewed where relevant
- no unbounded list paths or easy N+1 regressions in touched code

### Technical Debt Analysis

Goal: structural debt is identified, quantified, and tracked before it compounds into development friction or bugs.

Good evidence:
- god objects, duplicated logic, and boilerplate patterns identified with line counts and file locations
- severity assessed (critical/high/medium/low) based on active harm vs. friction
- concrete Linear issues created for each finding with actionable scope
- hacks, shortcuts, and open vulnerabilities surfaced with code references
- large files (>2K lines non-test) catalogued with the structural reason they grew

### Issue Tracking Hygiene

Goal: Linear reflects reality closely enough that active work is visible, stalled work is noticed, and release planning is not distorted by stale execution state.

Good evidence:
- OSS project issues already in `In Progress` were reviewed for stale ownership or stalled execution
- issues whose `updatedAt` was older than 1 day were triaged, commented, re-scoped, or moved out of `In Progress`
- maintenance findings that should not be fixed immediately were captured as actionable Linear issues or comments instead of left implicit

### Repo Workflow Hygiene

Goal: agent instructions, commands, skills, examples, and release helpers still match reality.

Good evidence:
- `AGENTS.md`, `.claude/commands/`, and `.claude/skills/` do not contradict each other
- release or maintenance instructions point at the canonical workflow instead of duplicating stale detail
- `.agents/plugins/marketplace.json`, `.claude-plugin/marketplace.json`, `plugins/everruns-dev/.codex-plugin/plugin.json`, `plugins/everruns-dev/.claude-plugin/plugin.json`, and shipped plugin behavior were checked against recent upstream plugin-platform changes; registration, version parity, compatibility, or discoverability gaps were fixed or captured

## Common Evidence Commands

Pick only what matches the task:

- `just pre-push`
- `just pre-pr`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-features`
- `cd apps/ui && npm run lint && npm run build`
- `cd apps/docs && npm run build`
- `./scripts/export-openapi.sh`
- `doppler run -- bash -lc 'GH_TOKEN="$GITHUB_TOKEN" gh api repos/everruns/everruns/dependabot/alerts --jq "[.[] | select(.state==\"open\")] | length"'` — open Dependabot alert count
- `doppler run -- bash -lc 'GH_TOKEN="$GITHUB_TOKEN" gh api repos/everruns/everruns/secret-scanning/alerts --jq "[.[] | select(.state==\"open\")] | length"'` — open secret scanning alert count
- `scripts/test-everruns-dev-plugin.sh` — Everruns Dev plugin metadata, registration, and version parity
- Linear MCP: list OSS project issues in `In Progress`, compare each issue's `updatedAt` to current time, and flag items older than 1 day for triage

## Deliverable

Report:

- what scope was covered
- what was fixed or found
- what evidence was gathered
- which stale `In Progress` Linear issues were triaged, if that check was in scope
- what was intentionally skipped and why

If the user asks to ship after maintenance, hand off to [`/ship`](../ship/SKILL.md).
