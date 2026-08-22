---
name: ship
description: Goal-oriented workflow for landing a requested change safely. Use when the user asks to ship, fix and ship, take a change through validation, or drive PR/CI/merge to completion.
metadata:
  internal: true
user-invocable: true
---

# Ship

Land the requested change with evidence, and merge only after CI is green.

Read [`knowledge/project/shipping.md`](../../../knowledge/project/shipping.md) first — it owns the success bar, the
constraints, the CI opt-out labels, and the reporting standard. This skill owns execution only.
Start from the goal and the changed risk surface, then take the shortest path that proves the
change is ready; do not walk this file as a fixed checklist.

## Sequence

1. **Check the branch.** Never ship from `main`. Clean tree before the final push.
2. **Review the delta.** `git diff origin/main...HEAD` and `git log origin/main..HEAD`. Confirm the
   requested behavior is actually implemented, then simplify avoidable complexity you introduced.
3. **Validate to the risk.** Pick from the evidence commands below; deepen only where signal is
   weak. Bug fixes prefer a failing test before the fix.
4. **Security review.** Mandatory for any change touching code, config, or infrastructure. Follow
   [`references/security-review.md`](references/security-review.md).
5. **Sync artifacts** the change actually affects: `knowledge/`, `knowledge/security/threat-model.md`, `AGENTS.md`,
   `test_cases/`, `apps/docs/`, OpenAPI exports.
6. **Smoke test the affected flows** end to end. For a coding-agent stack, follow the canonical
   startup contract in the root `AGENTS.md`; it includes the DB-backed infrastructure required for
   database, migration, infra, or API integration risk.
   Stop anything you started. Docs- or config-only changes may skip this with a stated reason.
7. **Decide follow-ups explicitly.** Prefer implementing in-scope work now. List anything deferred
   under **Follow-ups** in the PR body with a one-line rationale, or state "No follow-ups." — a
   reader must be able to tell "nothing left" from "agent forgot". File Linear issues (OSS/EVE) for
   non-trivial deferrals.
8. **Open the PR and merge** — see [`references/pr-and-merge.md`](references/pr-and-merge.md).

## Migrations

Rebase silently keeps two branches' identical migration numbers. Run
`bash scripts/lib/check-migration-ordering.sh` after every rebase, and again immediately before
`gh pr merge` — another PR may have merged a colliding number during review. Renumber to the next
free number when it fails.

Merging without a final rebase is fine when it saves a CI cycle and no migration risk exists:

```bash
git fetch origin main
base="$(git merge-base HEAD origin/main)"
git diff --name-only "$base"..HEAD -- crates/server/migrations/
git diff --name-only "$base"..origin/main -- crates/server/migrations/
```

If either side touched migrations, rebase and re-run the ordering check. After merging without a
rebase, watch main CI for the merge commit.

## Evidence commands

Pick only what matches the changed surface:

- `just pre-push` / `just pre-pr`
- `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --all-features`, `cargo fetch --locked`
- `cd apps/ui && pnpm run format:check && pnpm run lint && pnpm run build`
- `cd apps/docs && pnpm run check && pnpm run build`
- `./scripts/export-openapi.sh`, then `cd apps/ui && pnpm run api-types:generate` —
  the UI's generated types are derived from the exported spec and CI fails on
  drift, so exporting without regenerating them is a guaranteed red build
- `bash scripts/lib/check-migration-ordering.sh`

If `just fmt` can auto-fix a failing format check, use it once and retry.

## Stopping

Stop and report only for blockers you cannot resolve safely alone: merge conflicts you cannot
adjudicate, missing credentials, ambiguous product intent, or CI failures you cannot reproduce.
