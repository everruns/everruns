---
type: Specification
title: "Shipping Specification"
description: "Goal-oriented shipping and merge-readiness guidance."
tags:
  - everruns
  - project
---
# Shipping Specification

## Abstract

This specification defines goal-oriented shipping for Everruns. Shipping should complete the requested goal, gather convincing evidence, create a mergeable PR, and merge only after CI is green.

The canonical agent workflow lives in [`.agents/skills/ship/SKILL.md`](../../.agents/skills/ship/SKILL.md). That skill is intentionally user-invocable so shipping can be requested directly as `/ship`.

## Design Goals

Shipping work should optimize for these outcomes:

1. Reach the requested goal, not just perform rituals around it.
2. Match validation depth to the actual risk surface.
3. Keep affected artifacts and release metadata in sync.
4. Merge only from a safe branch state with green CI.

## Ownership Boundary

- This spec owns the shipping intent, constraints, and success bar.
- The skill owns the execution workflow, heuristics, and example commands.
- Other specs remain the source of truth for their domains. Shipping should update them when the change affects those domains rather than re-describe them here.

Relevant references:
- [`knowledge/project/release-process.md`](release-process.md)
- [`knowledge/security/threat-model.md`](../security/threat-model.md)
- [`knowledge/foundations/architecture.md`](../foundations/architecture.md)
- [`knowledge/project/commands.md`](commands.md)
- [`knowledge/project/skills-registry.md`](skills-registry.md)

## Required Outcomes

Every shipped change satisfies all of these outcomes. They are the success bar, not a menu.

1. Safe branch state: no shipping from `main` or `master`; working tree clean before final push; prefer rebasing onto the latest `origin/main` before merge. Merging without the latest rebase is allowed when saving another CI cycle is more valuable and migration risk is absent: fetch `origin/main`, verify neither `origin/main` nor the PR changed `crates/server/migrations/` since their merge base, and monitor main CI after merge. If either side changed migrations, rebase and run `bash scripts/lib/check-migration-ordering.sh` to verify migration numbers are strictly sequential. Re-run the same check immediately before `gh pr merge`, since other PRs may have merged a colliding number while yours was in review. Renumber your migration if a conflict exists.
2. Goal achieved with evidence: the requested behavior is implemented and validated with proof that matches the risk.
3. Merge-ready code: touched code is reviewed for avoidable complexity and performance risk. A structured security review is performed against the relevant threat model categories from `knowledge/security/threat-model.md` (procedure: [`.agents/skills/ship/references/security-review.md`](../../.agents/skills/ship/references/security-review.md)). Issues found during review are addressed or explicitly blocked.
4. Synced artifacts: only the affected artifacts are updated, including specs, threat model, docs, OpenAPI, test cases, and agent instructions when relevant. Specs touched by the change must not contain implementation details that duplicate code (struct fields, enum variants, exhaustive tables, code snippets), replace with links to source files per the spec hygiene guidance in `knowledge/index.md`.
5. Smoke test impacted functionality: always smoke test the flows affected by the change end-to-end. This is mandatory, not conditional on risk assessment. Docs-only or config-only changes that do not affect runtime behavior may skip smoke testing with explicit justification.
6. Follow-ups surfaced: the agent actively looks for in-scope work that risks being silently dropped (TODOs, partial fixes, declined suggestions, missed edge cases, spec/doc drift) and prefers to implement them in this PR. Anything deferred is listed under a **Follow-ups** section in the PR body with a one-line rationale; if nothing is deferred, the PR body must explicitly state "No follow-ups." so readers can distinguish completeness from omission.
7. Safe merge: the PR uses the repo template, CI is green, no temporary `ci:skip-*` opt-out label suppresses CI affected by the PR diff, **every** review comment from all reviewers (including async bot reviewers and low-confidence suggestions) is explicitly analyzed, reasoned about, and resolved, either with a code change or a written explanation, after a final post-green sweep, merge happens with squash only, and main CI is monitored after merge.

## Constraints

- Shipping is outcome-oriented, not a mandatory linear checklist.
- Validation should start with the smallest high-signal proof and deepen only when risk or weak signals require it.
- Bug fixes should prefer a failing test before the fix when practical, but the validation strategy may vary when a smaller or stronger proof exists.
- Docs-only or config-only changes may skip code tests if that choice is justified and the relevant docs or build checks were run.
- CI is slow enough that reducing unnecessary cycles is important. Temporary CI opt-out labels may skip expensive interim PR checks to conserve CI capacity, but they are not merge evidence for affected surfaces. Before merge, opt-outs must not suppress CI checks affected by the PR diff, and merge must wait for those affected checks to pass.
- A latest-main rebase is preferred but not mandatory when migration risk is absent. If merging without it, explicitly verify no migration changes exist on either side since the merge base and watch main CI after merge.
- Security review is mandatory for all code, configuration, and infrastructure changes. The review must identify relevant threat model categories, check the diff against them, and document findings. Perceived low risk does not justify skipping the review.
- Every review comment must be explicitly addressed before merge, including low-confidence suggestions, nits, and bot comments. For each comment, the agent must analyze the concern, reason about whether a change is warranted, and either apply a fix or reply with a clear explanation. Dismissing or ignoring comments without reasoning is not permitted.
- Auto-merge must not bypass the final review pass; shipping should give async reviewer bots time to comment after the last push and after CI turns green, then re-check before merge.
- If a blocker cannot be resolved safely by the agent alone, shipping must stop and report the blocker rather than guess.

## Evidence

The command menu lives in the ship skill; it picks the smallest set that fits the changed surface
rather than running every check mechanically. Whatever set is chosen, the evidence must show that:

- the changed surface built, linted, and tested cleanly, including important negative paths
- impacted flows were smoke tested against the repository's canonical local startup contract
- performance impact was considered where relevant: indexes, scans, N+1 patterns, pagination, and
  bounded result sets
- UI changes were captured as screenshots or recordings in validation or PR comments, never
  committed to the repo

## CI Opt-Out Labels

Long CI jobs can be skipped on interim PR pushes with temporary labels:

- `ci:skip-docker` skips PR Docker image builds.
- `ci:skip-slow-rust` skips PostgreSQL integration tests, release binary builds, workflow tests, CLI E2E, OpenAPI freshness, and dependent SDK checks.
- `ci:skip-postgres-integration` skips only the main PostgreSQL integration test job.
- `ci:skip-sdk-compat` skips SDK compatibility checks only.
- `ci:skip-ui-e2e` skips Playwright smoke tests; UI build/lint/unit tests still run.
- `ci:skip-docs-notebooks` skips executable docs notebooks; docs check/build still run.
- `ci:skip-integration-workflows` skips the standalone PR integration workflows for Brave Search, DuckDuckGo, and Parallel MCP.

Use these labels to save iteration time after deciding the skipped surface is low-signal for the current push. The CI Opt-Out Policy job fails only when an opt-out label suppresses CI affected by the PR diff. Before merge, remove any CI opt-out label that blocks affected checks and rerun CI on the final PR commit so the affected checks run green.

## Reporting Standard

Shipping output should make it easy to evaluate readiness:

- what changed
- what evidence was gathered
- security review: which threat categories were checked, any findings, and how they were resolved
- what was skipped during iteration and why, plus confirmation that final merge CI ran the checks affected by the PR diff
- whether the PR was rebased onto latest main; if not, the migration-change check and main-CI monitoring outcome
- review comments: how each comment was addressed (code change or written reasoning)
- follow-ups: what was deferred and why (or an explicit "No follow-ups." statement)
- what blockers or residual risks remain
