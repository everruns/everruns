# Linear Issues Processing Specification

## Abstract

This specification defines the workflow for processing Linear issues using coding agents. Each open issue is picked up, fixed with extensive test coverage, documented, and shipped as an individual PR that is merged when CI is green.

## Requirements

### Prerequisites

- Linear MCP server configured (`.mcp.json`)
- `LINEAR_API_KEY` available via Doppler
- GitHub CLI authenticated: `doppler run -- bash -lc 'GH_TOKEN="$GITHUB_TOKEN" gh auth status'`

### Workflow per Issue

For each open Linear issue, execute the following steps in order:

#### 1. Pick Up Issue

1. Query open issues from Linear via MCP (filter by team/project as appropriate)
2. Read issue title, description, labels, priority, and any linked PRs
3. Mark issue as **In Progress** in Linear
4. Assign to current agent/user if unassigned

#### 2. Analyze and Plan

1. Parse issue description for reproduction steps, expected behavior, and acceptance criteria
2. Search codebase for relevant files using issue keywords
3. Identify root cause (for bugs) or implementation scope (for features)
4. Create a todo list with actionable steps
5. If requirements are ambiguous, add a comment on the Linear issue requesting clarification and move to the next issue

#### 3. Create Branch

1. Branch from `main`: `git checkout -b claude/{issue-id}-{short-description}`
2. Branch name must use lowercase kebab-case

#### 4. Implement Fix

1. **Write failing test first** (for bugs: reproduces the issue; for features: validates acceptance criteria)
2. Implement the minimal fix or feature
3. Avoid over-engineering: only change what the issue requires
4. Follow project style (see `specs/code-organization.md`)

#### 5. Test Coverage

Every fix must include extensive test coverage:

**Unit Tests:**
- Test the changed function/module directly
- Cover happy path, error cases, and edge cases
- For bug fixes: test must fail without the fix applied
- For features: test must cover all acceptance criteria
- Aim for coverage of all touched code paths

**Integration Tests:**
- Test the change through the service layer or API boundary
- Verify side effects (database state, events emitted, etc.)
- Test interaction with adjacent components
- For API changes: test request validation, response shape, error codes

**Test Naming:**
- `test_{function}_{scenario}_{expected}` (e.g., `test_create_agent_duplicate_name_returns_conflict`)
- Tests must be deterministic and independent

#### 6. Update Specs and Docs

If the change affects documented behavior:

1. Update relevant specs in `specs/` to reflect new behavior
2. Update OpenAPI spec: `./scripts/export-openapi.sh`
3. Update docs in `apps/docs/` if user-facing behavior changes
4. Update `AGENTS.md` if development workflow or tooling changes
5. Update test cases in `test_cases/` if manual test steps change

#### 7. Pre-PR Validation

Run the full pre-PR checklist:

1. `cargo fmt --check`
2. `cargo clippy --all-targets --all-features -- -D warnings`
3. `cargo test --all-features`
4. `npm run lint` + `npm run build` in `apps/ui/` (if UI touched)
5. `./scripts/export-openapi.sh` (if API touched)
6. `npm run build` in `apps/docs/` (if docs touched)
7. Rebase on main: `git fetch origin main && git rebase origin/main`

Or use shorthand: `just pre-pr`

#### 8. Create PR

1. Push branch: `git push -u origin <branch-name>`
2. Create PR using `.github/pull_request_template.md`:
   - **What**: Description of the change
   - **Why**: Link to Linear issue (e.g., `Fixes LIN-123`)
   - **How**: High-level approach
   - **Risk**: Low / Medium / High with what can break
   - **Checklist**: Tests added, specs updated
3. Commit format: `fix(scope): description` or `feat(scope): description`
4. Use `chore` type for spec-only or doc-only changes
5. Never add Claude session links to PR body or commits

#### 9. Merge When Green

1. Wait for CI to pass (all GitHub Actions checks green)
2. **Never merge when CI is red** — no exceptions
3. If CI fails: fix the issue, push again, wait for green
4. Merge strategy: **Squash and Merge**

#### 10. Close Issue

1. Mark issue as **Done** in Linear
2. Add a comment on the Linear issue linking to the merged PR
3. Move to next open issue

### Issue Prioritization

Process issues in this order:

1. **Urgent/Critical** priority first
2. **High** priority
3. **Medium** priority
4. **Low** priority
5. Within same priority: oldest first (FIFO)

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
