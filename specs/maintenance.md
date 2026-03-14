# Maintenance Specification

## Abstract

This specification defines goal-oriented maintenance for Everruns. Maintenance should improve release readiness and repo health with evidence, not by mechanically executing a fixed checklist.

The canonical agent workflow lives in [`.claude/skills/maintenance/SKILL.md`](../.claude/skills/maintenance/SKILL.md). That skill is intentionally user-invocable so maintenance can be requested directly as `/maintenance`.

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
- [`specs/code-organization.md`](./code-organization.md)
- [`specs/commands.md`](./commands.md)
- [`specs/skills-registry.md`](./skills-registry.md)

## Constraints

- Maintenance is risk-proportional, not sweep-proportional. A larger checklist is not inherently better.
- The selected maintenance scope must be explained, including what was skipped and why.
- If maintenance changes code or behavior, affected artifacts must stay in sync: specs, docs, OpenAPI, threat model, test cases, agent instructions, and release materials as applicable.
- Specs should not duplicate operational workflow text that belongs in skills or commands.
- Maintenance should prefer concrete fixes over ceremonial audits when a safe local fix exists.

## Release Readiness Standard

Before a release, maintenance should cover:

- areas changed since the last release
- historically fragile or high-risk surfaces
- release artifacts affected by those changes

A full-repo sweep is not mandatory if the evidence is already strong. The bar is confidence, not checklist completion theater.

## Reporting Standard

Maintenance output should make it easy to evaluate readiness:

- scope covered
- fixes made or findings recorded
- evidence gathered
- skipped areas and rationale

## Frequency

Run maintenance before each release and whenever repo health visibly drifts. Sections can be handled independently during normal development.
