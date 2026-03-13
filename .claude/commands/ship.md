# Ship

Land a change safely: achieve the requested goal, gather convincing evidence, create a mergeable PR, and merge only after CI is green.

This command implements the "Shipping" definition and Pre-PR Checklist from AGENTS.md. Optimize for required outcomes, not a rigid script. Use judgment on ordering and depth: if risk is low, keep it lean; if risk is high, increase validation.

## Arguments

- `$ARGUMENTS` - Optional: description of what is being shipped (used for PR title/body context and to scope the quality checks)

## Instructions

### Required outcomes

1. **Safe branch state**
   - Not on `main` or `master`
   - Clean working tree before final push
   - Rebased onto the latest `origin/main` before merge
2. **Goal achieved with evidence**
   - Review the delta with `git diff origin/main...HEAD` and `git log origin/main..HEAD`
   - Confirm the requested behavior is implemented
   - Add or update validation that matches risk:
     - Bug fixes: prefer a failing test first, then prove the fix
     - Features: cover acceptance criteria plus important negative paths
     - Docs/config-only changes: explain why code tests are unnecessary and run the relevant docs/build/lint checks
3. **Changed code is fit to merge**
   - Simplify obvious duplication or unnecessary complexity
   - Review touched areas for security risks: auth, input validation, data exposure, injection, dependency risk
   - If you find issues, fix them and refresh the validation
4. **Artifacts stay in sync**
   - Update only the artifacts affected by the change:
     - `specs/`
     - `specs/threat-model.md`
     - `AGENTS.md`
     - `test_cases/`
     - `apps/docs/`
     - OpenAPI via `./scripts/export-openapi.sh`
5. **Relevant runtime confidence exists**
   - Smoke test impacted flows when tests/builds are not enough
   - Prefer dev mode for fast UI checks: `just start-dev --no-watch`
   - Use full mode for database, migration, infra, or API integration risk: `just start-all --no-watch`
   - Stop any servers you started
6. **PR is created and merged safely**
   - Push the branch
   - Create or update the PR using `.github/pull_request_template.md`
   - Wait for CI to go green
   - Merge with squash only after green CI

### Hints

- Start from the goal and the risk surface. Touching auth, persistence, migrations, external APIs, or end-to-end UI flows usually warrants deeper validation.
- Use the smallest set of checks that gives high confidence. Expand only when the first pass leaves gaps or reveals issues.
- If a bug is hard to capture with a unit test, choose the next-best proof: integration test, regression harness, or explicit smoke test.
- If `just fmt` can auto-fix a failing formatting check, use it and retry once.
- Stop only for blockers you cannot safely resolve alone, such as merge conflicts, missing credentials, or ambiguous product intent.

### Common checks by impact

Run the checks that match the files and risk you changed. Skip unaffected categories.

| Check | Run when |
|-------|----------|
| `cargo fmt --check` | Rust or workspace config changed |
| `cargo clippy --all-targets --all-features -- -D warnings` | Rust changed |
| `cargo test --all-features --lib --bins` | Rust changed |
| `cargo fetch --locked` | Rust dependencies or lockfile changed |
| `cd apps/ui && npm run format:check && npm run lint && npm run build` | UI changed |
| `./scripts/export-openapi.sh` | API surface changed |
| `cd apps/docs && npm run check && npm run build` | Docs changed |

Before the final quality pass and PR merge, rebase:

```bash
git fetch origin main && git rebase origin/main
```

If rebase fails with conflicts, stop and report it.

### PR and merge

Use conventional-commit style for the PR title. In the body, explain:

- what changed
- why it changed
- how you validated it
- notable risks or follow-ups

Use `gh pr view --json url` to detect an existing PR. Create one with `gh pr create` if needed. Poll `gh pr checks` until CI finishes. If CI is green, merge with `gh pr merge --squash --auto`. If CI is red, report the failures and stop.

## Notes

- This is the canonical shipping workflow. It implements the "Shipping" definition and Pre-PR Checklist from AGENTS.md.
- The quality bar is mandatory. The exact order is flexible.
- The `$ARGUMENTS` context helps scope the goal, risk, validation, and artifact updates.
- For "fix and ship" requests: implement the fix first, then run `/ship` to validate and merge.
