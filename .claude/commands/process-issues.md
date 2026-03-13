# Process Issues

Process open Linear issues in the **OSS** project: pick up, fix, ship, close. One PR per issue, max 5 issues in parallel.

Full workflow design and rationale: `specs/linear-issues.md`.

## Arguments

- `$ARGUMENTS` - Optional: specific issue IDs, priority filter, or project override (defaults to OSS project)

## Instructions

### Phase 1: Prerequisites

Verify environment:

```bash
doppler run -- env | rg 'LINEAR_API_KEY|GITHUB_TOKEN'
doppler run -- bash -lc 'GH_TOKEN="$GITHUB_TOKEN" gh auth status'
```

Linear MCP server must be configured in `.mcp.json`.

### Phase 2: Query and Prioritize

1. Query open issues from the **OSS** project in Linear via MCP (apply `$ARGUMENTS` filters if provided)
2. Read each issue's title, description, labels, priority, and linked PRs
3. Sort by priority (Urgent > Critical > High > Medium > Low), then oldest first within same priority
4. Select up to **5 issues** to process in parallel

### Phase 3: Process Each Issue

For each selected issue, run these steps. Process up to 5 issues concurrently using subagents.

#### 3a. Pick Up

1. Mark issue as **In Progress** in Linear
2. Assign to current agent/user if unassigned

#### 3b. Analyze

1. Parse issue for reproduction steps, expected behavior, acceptance criteria
2. Search codebase for relevant files
3. Identify root cause (bugs) or implementation scope (features)
4. If requirements are ambiguous: add a Linear comment requesting clarification, skip this issue

#### 3c. Branch

```bash
git fetch origin main && git checkout -b {issue-id}-{short-description} origin/main
```

- `{issue-id}`: Linear issue identifier (e.g., `ENG-123`)
- `{short-description}`: kebab-case summary (e.g., `fix-session-timeout`)

#### 3d. Implement and Ship

Run `/ship` to implement, test, validate, create PR, and merge. Pass the Linear issue context:

- **What to implement**: the issue description and acceptance criteria
- **PR "Why" section**: link to Linear issue (e.g., `Fixes ENG-123`)
- **Write failing test first** for bugs; validate acceptance criteria for features

`/ship` handles the required shipping outcomes: evidence for correctness, code simplification, security review, artifact updates, smoke testing, quality gates (including rebase on main), PR creation, CI wait, and merge.

#### 3e. Merge PRs Sequentially

When multiple issues produce PRs, merge them one at a time with rebase between each:

1. Merge the first PR (CI must be green)
2. Before merging the next PR, rebase it onto the updated `main`: `git fetch origin main && git rebase origin/main`
3. Push the rebased branch and wait for CI to pass
4. Repeat until all PRs are merged

This prevents combined-merge breakage (e.g., duplicate Cargo.toml workspace deps, overlapping file edits).

#### 3f. Close Issue

After PR is merged:

1. Mark issue as **Done** in Linear
2. Add a comment on the Linear issue linking to the merged PR

### Phase 4: Report

After all issues are processed, report:

- Issues completed (with PR links)
- Issues skipped (with reasons)
- Issues that failed (with error details)

## Skipping Rules

Skip an issue and move to the next if:

- Blocked by external dependency (add comment, leave as In Progress)
- Requirements ambiguous and clarification pending
- Requires access/permissions not available

Always add a Linear comment explaining why.

## Notes

- One PR per issue, always. No batch PRs.
- No partial fixes — fully resolve or skip.
- Max 5 issues in parallel to avoid resource contention.
- See `specs/linear-issues.md` for design rationale, prioritization details, and non-requirements.
