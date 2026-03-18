---
name: process-issues
description: Process open Linear issues — pick up, fix, and ship one PR per issue. Use when the user asks to process issues, work on Linear issues, tackle the backlog, or fix open issues.
user-invocable: true
---

# Process Issues

Goal: resolve open Linear issues from the **OSS** project by shipping one merged PR per issue, with full `/ship` quality.

This skill implements [`specs/linear-issues.md`](../../../specs/linear-issues.md). Keep operational guidance here. Keep design intent and constraints in the spec.

This skill is outcome-oriented. Do not blindly walk a fixed checklist. Start from the issue backlog, pick the highest-value work, and drive each issue to a merged PR.

## When To Use

Use this skill when the user asks to:

- process issues or work on Linear issues
- tackle the backlog or fix open issues
- pick up and ship Linear issues
- work through outstanding bugs or features

## Arguments

- `$ARGUMENTS` — Optional: specific issue IDs, priority filter, or project override (defaults to OSS project)

## Required Outcomes

1. **One PR per issue, always.**
   - Every issue gets its own branch and PR. No batch PRs. No partial fixes.
   - Branch naming: `{issue-id}-{short-description}` from `origin/main` (lowercase kebab-case).
   - Each PR must satisfy the full `/ship` outcomes: goal met, validation matches risk, artifacts updated, CI green, PR merged safely.

2. **Issues are fully resolved or explicitly skipped.**
   - Resolved: PR merged, issue marked **Done** in Linear with a comment linking the merged PR.
   - Skipped: Linear comment explaining why (blocked, ambiguous requirements, missing access). Issue left as **In Progress** or unchanged.

3. **Sequential merging prevents conflicts.**
   - After merging each PR, rebase remaining open PRs onto updated `main` before merging the next.
   - Never merge a PR whose CI ran against a stale base.

4. **A summary report is delivered.**
   - Issues completed (with PR links)
   - Issues skipped (with reasons)
   - Issues that failed (with error details)

## Operating Model

### Prerequisites

Verify environment before starting:

```bash
doppler run -- env | rg 'LINEAR_API_KEY|GITHUB_TOKEN'
doppler run -- bash -lc 'GH_TOKEN="$GITHUB_TOKEN" gh auth status'
```

Linear MCP server must be configured in `.mcp.json`.

### Query and Prioritize

1. Query open issues from the **OSS** project via Linear MCP (apply `$ARGUMENTS` filters if provided)
2. Read each issue's title, description, labels, priority, and linked PRs
3. Sort by priority (Urgent > Critical > High > Medium > Low), then oldest first within same priority
4. Select up to **5 issues** to process

### Per-Issue Execution

For each issue, process up to 5 concurrently using subagents:

1. **Pick up** — mark as **In Progress** in Linear, assign if unassigned
2. **Analyze** — parse requirements, search codebase, identify root cause or scope. If ambiguous: comment in Linear requesting clarification, skip
3. **Branch** — `git fetch origin main && git checkout -b {issue-id}-{short-description} origin/main`
4. **Ship** — delegate to [`/ship`](../ship/SKILL.md) with the issue context:
   - What to implement: issue description and acceptance criteria
   - PR links back to the Linear issue (e.g., `Fixes ENG-123`)
   - Write failing test first for bugs; validate acceptance criteria for features
5. **Close** — after PR merge: mark issue **Done**, add Linear comment with PR link

### Merge Sequencing

When multiple issues produce PRs, merge one at a time:

1. Merge first PR (CI must be green)
2. Rebase next PR onto updated `main`, push, wait for CI
3. Repeat until all PRs are merged

### Skip Rules

Skip an issue if:

- Blocked by external dependency
- Requirements ambiguous and clarification pending
- Requires access/permissions not available

Always add a Linear comment explaining why.

## Constraints

- Max 5 issues in parallel
- No batch PRs — one PR per issue
- No partial fixes — fully resolve or skip
- See `specs/linear-issues.md` for full design rationale
