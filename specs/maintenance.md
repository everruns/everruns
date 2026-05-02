# Maintenance Specification

## Abstract

This specification defines goal-oriented maintenance for Everruns. Maintenance should improve release readiness and repo health with evidence, not by mechanically executing a fixed checklist.

The canonical agent workflow lives in [`.agents/skills/maintenance/SKILL.md`](../.agents/skills/maintenance/SKILL.md). That skill is intentionally user-invocable so maintenance can be requested directly as `/maintenance`.

## Design Goals

Maintenance work should optimize for these outcomes:

1. Make the maintenance scope explicit.
2. Improve the repo in concrete ways or produce crisp findings with evidence.
3. Match validation depth to the actual risk surface.
4. Keep release claims honest: do not call the repo ready unless the relevant surfaces were checked.

## Ownership Boundary

- This spec owns the maintenance intent, constraints, and success bar.
- The skill owns the execution workflow, heuristics, and example commands.
- Other specs remain the source of truth for their domains. Maintenance should update them when the corresponding behavior changes rather than re-describe those domains here.

Relevant references:
- [`specs/release-process.md`](./release-process.md)
- [`specs/threat-model.md`](./threat-model.md)
- [`specs/architecture.md`](./architecture.md)
- [`specs/commands.md`](./commands.md)
- [`specs/skills-registry.md`](./skills-registry.md)
- [`specs/issue-tracking.md`](./issue-tracking.md)
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
- When maintenance covers repo workflow hygiene, treat Codex and Claude Code plugin surfaces as maintained workflow artifacts. Review recent upstream plugin-platform changes before claiming either plugin is current, and verify the shared Everruns Dev plugin metadata stays in parity across Codex and Claude manifests.
- Specs should not duplicate operational workflow text that belongs in skills or commands.
- Maintenance should prefer concrete fixes over ceremonial audits when a safe local fix exists.

## Spec Hygiene

Specs must follow the **spec content principle** from `AGENTS.md`: they preserve design intent, rationale, and constraints — not implementation details readable from code. During maintenance, review specs in the changed surface for:

- **Struct field tables** that duplicate Rust struct definitions — replace with a link to the source file
- **Enum variant lists** that duplicate code — replace with a link
- **Exhaustive tables** (permissions, event types, feature flags, entity prefixes) that will drift as code evolves — replace with a link to the source of truth
- **Code snippets** that restate already-implemented logic (SQL DDL, macro expansions, error detection patterns) — replace with a link
- **Stale "current" lists** (e.g., "Current Flags" tables) — remove or replace with a link

The pattern: keep the "why" and constraints, link to code for the "what".

## Release Readiness Standard

Before a release, maintenance should cover:

- areas changed since the last release
- historically fragile or high-risk surfaces
- release artifacts affected by those changes
- the latest push-only integration live workflows on `main` plus the latest `.github/workflows/integration-live-sweep.yml` result; unresolved failures there block a "release ready" claim until triaged
- Linear issues already marked `In Progress` whose `updatedAt` is older than 1 day, signaling execution drift; use that `updatedAt` threshold as the default review threshold unless the task sets a stricter bar
- GitHub Security tab: security overview, Dependabot alerts, and open secret scanning alerts
- dependency versions across all packages (Cargo workspace crates, npm packages, CLI) checked for outdated major versions and deprecated crates
- the Codex and Claude Code plugin surfaces reviewed against recent upstream plugin-platform changes: inspect the latest relevant platform references, then compare them to `.agents/plugins/marketplace.json`, `.claude-plugin/marketplace.json`, `plugins/everruns-dev/.codex-plugin/plugin.json`, `plugins/everruns-dev/.claude-plugin/plugin.json`, and shipped plugin behavior; run `scripts/test-everruns-dev-plugin.sh` or equivalent metadata validation to prove registration and version parity before claiming release readiness

A full-repo sweep is not mandatory if the evidence is already strong. The bar is confidence, not checklist completion theater.

## Reporting Standard

Maintenance output should make it easy to evaluate readiness:

- scope covered
- fixes made or findings recorded
- evidence gathered
- skipped areas and rationale

## Frequency

Run maintenance before each release and whenever repo health visibly drifts. Sections can be handled independently during normal development.
