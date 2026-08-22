---
type: Specification
title: "Maintenance Specification"
description: "Goal-oriented maintenance and release-readiness guidance."
tags:
  - everruns
  - project
---
# Maintenance Specification

## Abstract

This specification defines goal-oriented maintenance for Everruns. Maintenance should improve release readiness and repo health with evidence, not by mechanically executing a fixed checklist.

The canonical agent workflow lives in [`.agents/skills/maintenance/SKILL.md`](../../.agents/skills/maintenance/SKILL.md). That skill is intentionally user-invocable so maintenance can be requested directly as `/maintenance`.

## Design Goals

Maintenance work should optimize for these outcomes:

1. Make the maintenance scope explicit.
2. Improve the repo in concrete ways or produce crisp findings with evidence.
3. Match validation depth to the actual risk surface.
4. Keep release claims honest: do not call the repo ready unless the relevant surfaces were checked.
5. Detect half-built features whose visible surfaces do not match their implementation status, especially gaps between UI, backend APIs, MCP, CLI, docs, and tests.
6. Keep shipped plugin surfaces current, mutually consistent, and aligned with upstream platform behavior.
7. Keep the size of shipped artifacts a maintained property rather than an accident, so release binaries and container images do not grow silently.

## Ownership Boundary

- This spec owns the maintenance intent, constraints, and success bar.
- The skill owns the execution workflow, heuristics, and example commands.
- Other specs remain the source of truth for their domains. Maintenance should update them when the corresponding behavior changes rather than re-describe those domains here.

Relevant references:
- [`knowledge/project/release-process.md`](release-process.md)
- [`knowledge/security/threat-model.md`](../security/threat-model.md)
- [`knowledge/security/security-testing.md`](../security/security-testing.md)
- [`SECURITY.md`](../../SECURITY.md)
- [`knowledge/foundations/architecture.md`](../foundations/architecture.md)
- [`knowledge/project/commands.md`](commands.md)
- [`knowledge/project/skills-registry.md`](skills-registry.md)
- [`knowledge/project/issue-tracking.md`](issue-tracking.md)
- [GitHub Security Overview](https://github.com/everruns/everruns/security)
- [Dependabot Alerts](https://github.com/everruns/everruns/security/dependabot)
- [Secret Scanning Alerts](https://github.com/everruns/everruns/security/secret-scanning?query=is%3Aopen+results%3Ageneric)
- [Claude Code Changelog](https://code.claude.com/docs/en/changelog)
- [Claude Code What's New](https://code.claude.com/docs/en/whats-new)
- [Claude Code Plugins Reference](https://code.claude.com/docs/en/plugins-reference)

## Constraints

- Maintenance is risk-proportional, not sweep-proportional. A larger checklist is not inherently better.
- The selected maintenance scope must be explained, including what was skipped and why.
- If maintenance changes code or behavior, affected artifacts must stay in sync: specs, docs, OpenAPI, threat model, test cases, agent instructions, and release materials as applicable.
- When maintenance covers repo workflow hygiene, treat Codex and Claude Code plugin surfaces as maintained workflow artifacts. Review recent upstream plugin-platform changes before claiming either plugin is current, verify the shared Everruns Dev plugin metadata stays in parity across Codex and Claude manifests, and resolve contradictions between plugin manifests, skills, MCP/app behavior, docs, and marketplace entries.
- Specs should not duplicate operational workflow text that belongs in skills or commands.
- Maintenance should prefer concrete fixes over ceremonial audits when a safe local fix exists.
- Dependency installs and updates must use a seven-day release-age floor across package ecosystems when the registry exposes publish timestamps. If the package manager cannot enforce that floor, maintenance must verify candidate release dates before accepting the update.

## Feature Completeness Drift

Maintenance should look for features that appear shipped in one surface but are missing, stubbed, or disconnected in another. A feature is not release-ready merely because one layer exists.

Examples of drift to catch:

- UI controls or pages that are not connected to backend state, mutations, streaming updates, auth, or error handling
- backend APIs with no reachable UI, CLI, MCP, SDK, or docs path when those surfaces are part of the intended product contract
- MCP tools, resources, or app cards that lag backend capabilities or expose behavior the UI/CLI cannot support consistently
- CLI commands that duplicate stale assumptions, omit important flags, or contradict current API semantics
- specs, docs, examples, tests, and manual test cases that describe a more complete feature than the product actually provides

The expected outcome is either a small fix that reconnects the surfaces or a crisp finding that names the missing surface, the user-visible impact, and the next action. Maintenance should not bury these gaps as generic technical debt because half-built features distort release readiness.

## Spec Hygiene

Specs in the changed surface must still meet the hygiene rules in [`knowledge/index.md`](../../knowledge/index.md),
which owns them. Copied implementation detail is a maintenance finding: replace it with a link to the
source of truth rather than re-syncing it.

## Release Readiness Standard

Before a release, maintenance should cover:

- areas changed since the last release
- historically fragile or high-risk surfaces
- release artifacts affected by those changes
- the latest push-only integration live workflows on `main` plus the latest `.github/workflows/integration-live-sweep.yml` result; unresolved failures there block a "release ready" claim until triaged
- Linear issues already marked `In Progress` whose `updatedAt` is older than 1 day, signaling execution drift; use that `updatedAt` threshold as the default review threshold unless the task sets a stricter bar
- GitHub Security tab: security overview, Dependabot alerts, and open secret scanning alerts
- threat model and security testing in sync per `knowledge/security/security-testing.md`: every `MITIGATED` threat has coverage where feasible, durable failpoint tests pass (`cargo test -p everruns-durable --test failure_injection_test --features "failpoints,postgres-tests" -- --test-threads=1`), and the `cargo deny check licenses` gate (`deny.toml`) passes. RustSec advisories are gated in CI by `cargo deny check advisories` for the workspace lockfile and by [`scripts/lib/check-nonworkspace-advisories.sh`](../../scripts/lib/check-nonworkspace-advisories.sh) for every committed lockfile outside it (workspace-excluded crates resolve separate dependency trees the workspace gate never sees). Dependabot alerts in the GitHub Security tab are a supplementary view, not the gate. A scheduled agent session cannot read the alert APIs directly — every request gets `403`, and swapping tokens does not help because the agent egress proxy rewrites `Authorization` for `api.github.com` and the session always authenticates as its own GitHub App installation (EVE-926) — so it reads them from the [`Security Alerts`](../../.github/workflows/security-alerts.yml) workflow instead, which reports into a job log the Actions API does expose. That recovers code-scanning alerts, which have no local gate at all; the Dependabot half stays blocked on `security_events` for that App installation, and the workflow says so rather than reporting an empty list
- DeepSec scan run for the changed surface, with deferred true-positive findings filed as issues (commands: [`.agents/skills/maintenance/references/surfaces.md`](../../.agents/skills/maintenance/references/surfaces.md)). The `.deepsec/` scanner is local dev-only tooling, exempt from the runtime seven-day dependency-maturity floor; bump it in its own commit rather than inside a product dependency bump
- dependency versions across all packages (Cargo workspace crates, pnpm-managed UI/docs packages, CLI) checked for outdated major versions and deprecated crates
  - `cargo outdated --root-deps-only --workspace` currently fails with a `libsqlite3-sys` `links` conflict because its temporary solve combines the workspace `sqlx 0.8` pin with the latest `rusqlite`. The real lockfile builds, so treat that failure as a tooling artifact and inspect `Cargo.toml` plus `cargo tree -i sqlx` / `cargo tree -i rusqlite` instead until `sqlx` and `rusqlite` are upgraded together
  - pnpm-managed packages enforce the seven-day release-age floor with `minimumReleaseAge: 10080`; do not bypass that gate for routine dependency maintenance
  - major pnpm upgrades (TypeScript, lucide-react, marked, openui packages) ship as separate PRs per framework family so each can be validated by the matching UI or docs build
  - bump pnpm packages that have transitive runtime dependencies (e.g. `@ag-ui/core`) together with the packages that pin them (e.g. `@openuidev/*`), otherwise pnpm installs duplicate copies in the lockfile
- shipped artifact size for the release binaries and container images, compared against the previous release. Growth without a named cause is a finding, not a rounding error; attribution tooling and its caveats live in [`.agents/skills/maintenance/references/surfaces.md`](../../.agents/skills/maintenance/references/surfaces.md)
- feature completeness across product surfaces: changed or recently shipped features should have their intended UI, backend, MCP, CLI, docs, tests, and manual-test coverage checked for disconnected, stubbed, or contradictory behavior
- crate documentation for every crate whose public surface changed: its `README.md` and crate-level rustdoc must still meet the crate documentation standard in `knowledge/foundations/code-organization.md` (abstract, ecosystem line, example, docs links, license; no publishing mechanics or internal `knowledge/` links; badges only on published crates)
- the Codex and Claude Code plugin surfaces reviewed against recent upstream plugin-platform changes: inspect the latest relevant platform references, then compare them to `.agents/plugins/marketplace.json`, `.claude-plugin/marketplace.json`, `plugins/everruns-dev/.codex-plugin/plugin.json`, `plugins/everruns-dev/.claude-plugin/plugin.json`, shipped plugin behavior, skills, docs, and marketplace entries; run `scripts/test-everruns-dev-plugin.sh` or equivalent metadata validation to prove registration, version parity, compatibility, and non-contradiction before claiming release readiness

A full-repo sweep is not mandatory if the evidence is already strong. The bar is confidence, not checklist completion theater.

## Reporting Standard

Maintenance output should make it easy to evaluate readiness:

- scope covered
- fixes made or findings recorded
- evidence gathered
- skipped areas and rationale

## Frequency

Run maintenance before each release and whenever repo health visibly drifts. Sections can be handled independently during normal development.
