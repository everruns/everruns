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
- Security alerts. **Do not call the alert APIs directly from a scheduled cloud-agent session** —
  `dependabot/alerts`, `secret-scanning/alerts` and `code-scanning/alerts` all answer `403` there,
  and no token changes that: the agent egress proxy rewrites `Authorization` for `api.github.com`,
  so the session always authenticates as its own GitHub App installation (EVE-926). Read the
  daily [`Security Alerts`](../../../.github/workflows/security-alerts.yml) job log over the
  Actions API instead, which carries code-scanning alerts and says explicitly that the Dependabot
  half is blocked:
  ```bash
  gh api "repos/everruns/everruns/actions/workflows/security-alerts.yml/runs?per_page=1" \
    --jq '.workflow_runs[0].id'
  gh api "repos/everruns/everruns/actions/runs/<run-id>/jobs" --jq '.jobs[].id'
  gh api "repos/everruns/everruns/actions/jobs/<job-id>/logs"
  ```
  Then confirm the locally gated half yourself, since that is where an actionable finding
  usually is: `cargo deny check advisories`, `bash scripts/lib/check-nonworkspace-advisories.sh`,
  and `pnpm audit --prod` in `apps/ui` and `apps/docs`.

## Deliverable

Report the scope covered, what was fixed or found, the evidence gathered, and what was intentionally
skipped and why. If the user then asks to ship, hand off to [`/ship`](../ship/SKILL.md).
