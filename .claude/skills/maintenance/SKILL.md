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
- Check OSS/EVE Linear issues already in `In Progress` when maintenance covers release readiness or workflow hygiene. Treat issues with no meaningful update for more than 2 days as stale by default, then triage or report them.
- Skip untouched areas when there is a clear reason. Say why they were skipped.
- Prefer fixing over reporting.
- For bugs uncovered during maintenance, prefer a failing test before the fix when practical.
- Keep changes PR-sized. If a maintenance theme explodes in scope, finish the highest-value slice and report the boundary.

## Maintenance Surfaces

Use judgment on which surfaces matter for the current task.

### Dependency Health

Goal: dependencies are current enough for the release risk, and upgrades do not silently break the repo.

Good evidence:
- lockfiles updated intentionally
- relevant build/lint/test checks pass
- major-version changes reviewed, not just applied blindly

### Specs And Docs Alignment

Goal: docs describe the current system intent and constraints, without drifting into code duplication.

Good evidence:
- changed behavior reflected in `specs/`, `apps/docs/`, OpenAPI, or examples when relevant
- stale or duplicate spec detail removed in favor of links to source files

### Security And Threat Posture

Goal: new or changed attack surface is understood, and mitigations/docs match reality.

Good evidence:
- threat model updated when behavior or trust boundaries changed
- obvious gaps in auth, validation, secret handling, or data exposure were reviewed
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
- OSS/EVE issues already in `In Progress` were reviewed for stale ownership or stalled execution
- issues with no meaningful update for more than 2 days were triaged, commented, re-scoped, or moved out of `In Progress`
- maintenance findings that should not be fixed immediately were captured as actionable Linear issues or comments instead of left implicit

### Repo Workflow Hygiene

Goal: agent instructions, commands, skills, examples, and release helpers still match reality.

Good evidence:
- `AGENTS.md`, `.claude/commands/`, and `.claude/skills/` do not contradict each other
- release or maintenance instructions point at the canonical workflow instead of duplicating stale detail

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
- Linear MCP: list OSS project issues in `In Progress`, compare `updatedAt` to current time, and flag items older than 2 days for triage

## Deliverable

Report:

- what scope was covered
- what was fixed or found
- what evidence was gathered
- which stale `In Progress` Linear issues were triaged, if that check was in scope
- what was intentionally skipped and why

If the user asks to ship after maintenance, hand off to [`/ship`](../ship/SKILL.md).
