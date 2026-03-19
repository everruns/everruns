---
name: ship
description: Goal-oriented workflow for landing a requested change safely. Use when the user asks to ship, fix and ship, take a change through validation, or drive PR/CI/merge to completion.
user-invocable: true
---

# Ship

Goal: land the requested change safely, with evidence, and merge only after CI is green.

This skill implements [`specs/shipping.md`](../../../specs/shipping.md). Keep operational guidance here. Keep the shipping success bar and constraints in the spec.

This skill is outcome-oriented. Do not blindly walk a fixed checklist. Start from the goal and changed risk surface, then choose the smallest path that proves the change is ready.

## When To Use

Use this skill when the user asks to:

- ship or fix and ship a change
- take work through validation, PR creation, CI, and merge
- prove a branch is merge-ready

## Required Outcomes

**ALL outcomes below are MANDATORY. These are not suggestions — do not skip or weaken any requirement.**

1. The branch state is safe.
   - Do not ship from `main` or `master`.
   - The working tree must be clean before the final push.
   - Rebase onto the latest `origin/main` before merge.
2. The requested goal is achieved with evidence.
   - Review the delta with `git diff origin/main...HEAD` and `git log origin/main..HEAD`.
   - Confirm the requested behavior is actually implemented.
   - Validation must match risk. For bugs, prefer a failing test first when practical.
3. The changed code is fit to merge.
   - Simplify obvious duplication or accidental complexity.
   - Review touched areas for auth, input validation, data exposure, injection, dependency, and performance risk.
   - Fix issues you find and refresh the evidence.
4. Relevant artifacts stay in sync.
   - Update only the artifacts affected by the change: `specs/`, `specs/threat-model.md`, `AGENTS.md`, `test_cases/`, `apps/docs/`, and OpenAPI exports when applicable.
5. Smoke test impacted functionality.
   - **Always** smoke test the flows affected by the change end-to-end. This is mandatory, not conditional on risk assessment.
   - Prefer `just start-dev --no-watch` for fast checks.
   - Use `just start-all --no-watch` when database, migration, infra, or API integration risk exists.
   - Stop any servers you started.
   - Docs-only or config-only changes that do not affect runtime behavior may skip smoke testing with explicit justification.
6. The PR is mergeable and merged safely.
   - Push the branch.
   - Create or update the PR with `.github/pull_request_template.md`.
   - Check the PR conversation, review threads, and review state from all reviewers, including bots.
   - After each push and again after CI turns green, wait at least 2 minutes for async reviewer bots to finish, then re-check for new comments before merge.
   - Address actionable review comments with code or doc changes, or reply with the resolution when no code change is needed.
   - Do not merge while substantive review feedback is still outstanding.
   - Wait for CI to go green.
   - Merge with squash only after CI is green and the final review/comment sweep above is clean.

## Operating Model

- Start from the goal and risk surface, not checklist order.
- Choose the highest-signal path first: targeted diff review, focused tests, relevant builds, then smoke tests if gaps remain.
- "Fix and ship" means implement first, then switch into shipping mode.
- Docs or config-only changes can skip code tests when you explain why and run the relevant docs, lint, or build proof.
- Do not use auto-merge or `gh pr merge --auto`; merge manually only after the final review sweep is clean because async review bots can post after the last push or after CI turns green.
- If `just fmt` can auto-fix a failing formatting check, use it once and retry.
- Stop only for blockers you cannot safely resolve alone: merge conflicts, missing credentials, ambiguous product intent, or CI failures you cannot reproduce or fix.

## Common Evidence Commands

Pick only what matches the changed surface:

- `just pre-push`
- `just pre-pr`
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-features`
- `cargo fetch --locked`
- `cd apps/ui && npm run format:check && npm run lint && npm run build`
- `./scripts/export-openapi.sh`
- `cd apps/docs && npm run check && npm run build`

## PR And Merge

- Use a conventional-commit style PR title.
- In the PR body, explain what changed, why it changed, how it was validated, and notable risks or follow-ups.
- Use `gh pr view --json url` to detect an existing PR.
- Create a PR with `gh pr create` if needed.
- Use `gh pr view --comments` to inspect the PR conversation, including bot comments.
- Use `gh pr view --json reviews,latestReviews` to inspect reviewer state.
- If review-thread status is unclear, inspect the review threads in the GitHub UI or via `gh api graphql` before merge.
- After the final push and after CI is green, wait at least 2 minutes for async reviewer bots, then do one last comment sweep before merge.
- Use `gh pr checks` to watch CI.
- Merge with `gh pr merge --squash` only after CI is green and the final review sweep is clean.
