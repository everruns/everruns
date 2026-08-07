---
name: maintenance
description: Goal-oriented repository maintenance and release-readiness work. Use when the user asks for maintenance, release prep, repo health review, dependency refreshes, spec/docs alignment, test gap review, technical debt analysis, or general cleanup without prescribing an exact sequence.
metadata:
  internal: true
user-invocable: true
---

# Maintenance

Leave the repo materially healthier and closer to release-ready, with evidence.

Read [`knowledge/project/maintenance.md`](../../../knowledge/project/maintenance.md) first — it owns the success bar, the
constraints, and the release-readiness standard. This skill owns execution only. Choose the smallest
set of actions that closes the real maintenance risk in front of you; a longer checklist is not a
better one.

## Sequence

1. **Fix the scope.** Use the user's scope if given. Otherwise infer one from recent diffs, release
   posture, and obviously stale areas — and state the assumption.
2. **Pick the highest-signal surface first.** Recent diffs, failing or flaky checks, stale specs,
   outdated dependencies, known security or performance hotspots. Skip untouched areas deliberately
   and say why. Surface-by-surface goals and evidence live in
   [`references/surfaces.md`](references/surfaces.md).
3. **Prefer fixing over reporting.** Fix what is small and local. For anything larger, produce a
   crisp finding with evidence and the next action, and file it in Linear (OSS project, EVE team).
   Bugs found here prefer a failing test before the fix.
4. **Validate to the risk.** Go deeper for auth, persistence, migrations, public API, external
   integrations, and end-to-end UI flows.
5. **Keep it PR-sized.** If a theme explodes, finish the highest-value slice and report the boundary.

Do not claim release readiness unless the changed and high-risk surfaces were actually checked.

## Evidence commands

Pick only what matches the scope:

- `just pre-push` / `just pre-pr`
- `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo test --all-features`
- `cd apps/ui && pnpm run lint && pnpm run build`
- `cd apps/docs && pnpm run build`
- `./scripts/export-openapi.sh`
- `scripts/test-everruns-dev-plugin.sh` — plugin metadata, registration, and version parity
- Dependabot / secret-scanning alert counts:
  ```bash
  doppler run -- bash -lc 'GH_TOKEN="$GITHUB_TOKEN" gh api repos/everruns/everruns/dependabot/alerts --jq "[.[] | select(.state==\"open\")] | length"'
  doppler run -- bash -lc 'GH_TOKEN="$GITHUB_TOKEN" gh api repos/everruns/everruns/secret-scanning/alerts --jq "[.[] | select(.state==\"open\")] | length"'
  ```

## Deliverable

Report the scope covered, what was fixed or found, the evidence gathered, and what was intentionally
skipped and why. If the user then asks to ship, hand off to [`/ship`](../ship/SKILL.md).
