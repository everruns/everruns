---
name: process-issues
description: Process open Linear issues — pick up, fix, and ship one PR per issue. Use when the user asks to process issues, work on Linear issues, tackle the backlog, or fix open issues.
user-invocable: true
---

# Process Issues

Goal: resolve open Linear issues from the **OSS** project by shipping one merged PR per issue, with full `/ship` quality.

This skill is outcome-oriented. Do not blindly walk a fixed checklist. Start from the issue backlog, pick the highest-value work, and drive each issue to a merged PR.

See [`specs/issue-tracking.md`](../../../specs/issue-tracking.md) for project scope and prerequisites.

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

2. **Each PR ships via the `/ship` skill.**
   - Always invoke `/ship` using the Skill tool — never duplicate its logic (validation, PR creation, CI checks, merge).
   - `/ship` owns the full shipping lifecycle: diff review, simplification, security review, artifact sync, evidence gathering, PR creation, CI wait, and squash merge.
   - Each PR must satisfy all `/ship` required outcomes: goal met, validation matches risk, artifacts updated, CI green, PR merged safely.

3. **Issues are fully resolved or explicitly skipped.**
   - Resolved: PR merged via `/ship`, issue marked **Done** in Linear with a comment linking the merged PR.
   - Skipped: Linear comment explaining why (blocked, ambiguous requirements, missing access). Issue left as **In Progress** or unchanged.

4. **Sequential merging prevents conflicts.**
   - After `/ship` merges each PR, rebase remaining open PRs onto updated `main` before shipping the next.
   - When multiple PRs touch `Cargo.toml` workspace deps, the second merge can create duplicate key errors — rebase catches this.
   - Never ship a PR whose CI ran against a stale base.

5. **A summary report is delivered.**
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
4. **Implement** — write the code change:
   - Parse issue description and acceptance criteria
   - Write failing test first for bugs; validate acceptance criteria for features
   - Commit with conventional-commit messages referencing the issue (e.g., `fix(core): resolve X — Fixes EVE-123`)
5. **Ship via `/ship` skill** — invoke `/ship` using the Skill tool to handle validation, PR creation, CI, and merge. **Do NOT** duplicate `/ship` logic (validation, PR creation, merge sequencing) inline — always delegate.
   - Pass context: what was implemented, which issue it resolves, the branch name
   - `/ship` owns: diff review, simplification, security review, artifact sync, evidence gathering, PR creation, CI wait, and merge
6. **Close** — after `/ship` completes merge: mark issue **Done** in Linear, add comment with merged PR link

### Merge Sequencing

When multiple issues produce PRs, merge sequentially (not in parallel):

1. Let `/ship` merge the first PR (CI must be green)
2. Rebase next PR onto updated `main`, push, then invoke `/ship` for that PR
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
- No manual deployment steps (CI/CD handles deployment)
- No backward compatibility shims (internal code, just change it)
- **No shipping shortcuts** — always invoke `/ship` via the Skill tool for validation, PR, CI, and merge. Never inline `/ship` logic or skip shipping steps
