---
name: process-issues
description: Process Linear issues — pick up, fix, and ship one PR per issue. Use when the user asks to process one issue, a specific Linear issue ID, multiple issues, the backlog, or outstanding bugs/features.
metadata:
  internal: true
user-invocable: true
---

# Process Issues

Resolve open Linear issues from the **OSS** project (EVE team) by shipping one merged PR per issue.
The same workflow applies whether the user names one issue or asks for the backlog.

`$ARGUMENTS` may carry issue IDs, a priority filter, or a project override.
[`knowledge/project/issue-tracking.md`](../../../knowledge/project/issue-tracking.md) has project scope and prerequisites.

## Rules that do not bend

- **One issue, one branch, one PR.** No batch PRs, no partial fixes.
- **[`/ship`](../ship/SKILL.md) does the shipping.** Invoke it via the Skill tool for every PR.
  Never re-implement its validation, PR, CI, or merge logic here.
- **Reproduce before fixing.** The issue text is a claim, not a spec.
- **Never take over someone else's issue silently.**

## Per issue

1. **Check ownership.** Read state, `updatedAt`, recent comments, and linked PRs.
   - `In Progress` and touched within 1 day → actively owned; skip, no comment.
   - `In Progress` and older than 1 day → raise the stale-ownership conflict to the human. Do not
     auto-claim, reassign, or reset.
   - Anything else → eligible.
2. **Claim it.** Move to **In Progress** and assign before any analysis or code, so humans can see
   the issue is owned.
3. **Doubt the issue.** Parse it, search the codebase, and decide whether the described problem is
   real, current, and correctly diagnosed. Ambiguous → comment asking for clarification, skip.
4. **Branch** from `origin/main` as `{issue-id}-{short-description}` (lowercase kebab-case).
5. **Reproduce against current `main`** — a failing test, a script, or a documented manual repro for
   bugs; a verified gap for features. Cannot reproduce → comment with what you tried and observed,
   then skip. Never patch symptoms you have not seen.
6. **Fix the root cause you reproduced**, not necessarily the fix the reporter prescribed. Note any
   divergence in the PR and in Linear. Commit as `fix(scope): … — Fixes EVE-123`.
7. **Ship** via `/ship`, passing what you implemented, which issue it resolves, and the branch.
8. **Close** — mark **Done** in Linear with a comment linking the merged PR.

Comment in Linear at milestones worth human attention: PR opened, blocker hit, scope decision, or
substantial CI/review follow-up. Keep them short.

## Batches

Up to 5 issues per run, processed concurrently with subagents. Sort eligible issues by priority
(Urgent > Critical > High > Medium > Low), then oldest first.

Merge sequentially, never in parallel: let `/ship` merge one PR, rebase the next onto updated `main`,
then ship it. Concurrent merges touching `Cargo.toml` workspace deps produce duplicate-key
conflicts, and a PR whose CI ran against a stale base is not evidence.

## Skips

Skip when the issue is actively owned, stale-owned pending a human decision, not reproducible,
blocked externally, ambiguous with clarification pending, or needs access you lack. Leave a Linear
comment explaining the skip only when this run actually investigated it — ownership-only skips go to
the human instead.

## Report

Issues completed with PR links, skipped with reasons, raised for stale-ownership decisions, and
failed with error details.

## Prerequisites

```bash
doppler run -- env | rg 'LINEAR_API_KEY|GITHUB_TOKEN'
doppler run -- bash -lc 'GH_TOKEN="$GITHUB_TOKEN" gh auth status'
```

The Linear MCP server must be configured in `.mcp.json`.
