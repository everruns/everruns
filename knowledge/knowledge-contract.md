---
type: Playbook
title: Knowledge Maintenance Contract
description: Rules for maintaining Everruns knowledge and OKF v0.2 conformance.
tags:
  - everruns
  - knowledge
  - okf
  - process
---

# Knowledge Maintenance Contract

`knowledge/` is Everruns' canonical [Open Knowledge Format (OKF) v0.2](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md)
bundle and persistent project memory. It owns durable design intent, constraints,
feature contracts, rationale, and success bars.

## Maintenance rules

- Treat knowledge as part of the implementation, not as historical documentation.
- Read the relevant concepts before changing behavior. Update them in the same change
  when decisions, behavior, constraints, threats, tests, or operations change.
- Record important decisions that are not recoverable from code. Prefer links to source
  and tests over duplicating volatile implementation details.
- Keep stable identifiers such as `TM-*`; never renumber them.
- Keep concepts readable in one sitting. Split an oversized concept by audience or
  subsystem and link its parts from the appropriate domain index.
- Public product documentation remains in `docs/`. Integration-specific specifications
  that live beside their implementation remain there and are linked from the relevant
  knowledge concept.

## Knowledge boundaries

A concept owns the **why** and the **what**: intent, rationale, constraints, contracts,
success bars, and rejected options. Everything else has a better home:

| Content | Source of truth |
|---|---|
| struct fields, enum variants, exhaustive tables, and ID prefixes | Rust source |
| SQL DDL, indexes, and triggers | `crates/server/migrations/` |
| request/response bodies and status codes | handlers and the OpenAPI export |
| protobuf messages | `crates/internal-protocol/proto/` |
| commands, flags, and procedures | `justfile`, scripts, or `.agents/skills/` |

Link to those sources instead of copying them. Stale copies are worse than no copy.

## OKF conformance

The bundle targets OKF v0.2, declared by `okf_version: "0.2"` in the root
[`index.md`](index.md).

- Every Markdown file except reserved `index.md` and `log.md` files is a concept and
  starts with YAML frontmatter containing a non-empty `type`.
- Concepts also carry `title` and single-sentence `description` metadata. `tags` are
  recommended.
- Domain `index.md` files have no frontmatter. The root index may contain only
  `okf_version` frontmatter.
- Every index is a link list that enumerates the concepts and immediate subdirectories
  beside it, and nothing deeper.
- `log.md` records date-grouped updates under `## YYYY-MM-DD`, newest first.
- Prose belongs in concepts, not indexes.
- Concept links are relative and must resolve.

Run `just check-okf` after changing the bundle. CI runs both the repository checker and
the pinned upstream `okf-lint` implementation.

## See also

- [Maintenance](project/maintenance.md), goal-oriented repository maintenance
- [Documentation](ui/documentation.md), the public documentation surface
- [Integration Specifications](integrations/integrations.md), specifications owned by
  integration crates
