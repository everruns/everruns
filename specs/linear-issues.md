# Linear Issues Processing Specification

## Abstract

This specification defines the workflow for processing Linear issues using coding agents. Each open issue is picked up, fixed with extensive test coverage, documented, and shipped as an individual PR that is merged when CI is green.

Executable workflow: `/process-issues` command (`.claude/commands/process-issues.md`).

## Project Scope

This repository manages issues in the **OSS** project within the Everruns Linear workspace. All new issues for this repo should be created in the OSS project. The `/process-issues` command queries the OSS project by default.

- **Linear workspace:** Everruns
- **Team:** EVE
- **Project:** OSS

## Requirements

### Prerequisites

- Linear MCP server configured (`.mcp.json`)
- `LINEAR_API_KEY` available via Doppler
- GitHub CLI authenticated: `doppler run -- bash -lc 'GH_TOKEN="$GITHUB_TOKEN" gh auth status'`

### Workflow Summary

Each issue follows: pick up → analyze → branch → implement → ship → close. The `/process-issues` command orchestrates this, delegating implementation and shipping to `/ship`.

**Concurrency:** process up to 5 issues in parallel.

**Branch naming:** `{issue-id}-{short-description}` from `main` (lowercase kebab-case).

### Merge Conflict Prevention

When processing multiple issues in parallel, PRs that merge independently can create conflicts on `main` (e.g., duplicate workspace dependencies, overlapping file edits). To prevent this:

1. **Sequential merging with rebase:** After merging each PR, rebase remaining open PRs onto the updated `main` before merging the next one. Do not merge multiple PRs without rebasing in between.
2. **Workspace dependency deduplication:** When multiple PRs add the same workspace dependency, the second merge creates a duplicate key error. After merging a PR that touches `Cargo.toml` workspace deps, check remaining PRs for conflicts and rebase them.
3. **CI validation after rebase:** After rebasing a PR onto updated `main`, wait for CI to pass before merging. Never merge a PR whose CI ran against a stale base.

### Issue Prioritization

Process issues in this order:

1. **Urgent/Critical** priority first
2. **High** priority
3. **Medium** priority
4. **Low** priority
5. Within same priority: oldest first (FIFO)

### Test Coverage Requirements

Every fix must include extensive test coverage:

- **Unit tests:** test changed function/module directly; cover happy path, error cases, edge cases; for bug fixes, test must fail without fix; for features, cover all acceptance criteria
- **Integration tests:** test through service layer or API boundary; verify side effects (database state, events emitted); test adjacent component interaction; for API changes, test request validation, response shape, error codes
- **Test naming:** `test_{function}_{scenario}_{expected}` (e.g., `test_create_agent_duplicate_name_returns_conflict`); tests must be deterministic and independent

### Skipping Issues

Skip an issue and move to the next if:

- Issue is blocked by an external dependency (add comment, leave as In Progress)
- Requirements are ambiguous and clarification is pending
- Issue requires access/permissions not available

Always add a Linear comment explaining why the issue was skipped.

## Non-Requirements

- No batch PRs (one PR per issue, always)
- No partial fixes (either fully resolve the issue or skip it)
- No manual deployment steps (CI/CD handles deployment)
- No backward compatibility shims (internal code, just change it)
