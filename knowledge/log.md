# Everruns Knowledge Update Log

## 2026-08-08

* **Platform resource grounding**: Distinguished operation discovery from
  authoritative entity reads, added user-scoped connection preflight, and
  required Platform Chat to report installed, available, attached, and connected
  integration state independently before reusable-resource confirmation.

## 2026-08-07

* **Slate fidelity**: Retired the handwritten experimental page stamp; experimental
  navigation now uses the single Lucide flask marker defined by the design system.

* **Platform behavioral eval**: Replaced the legacy tool-name study with a
  live-server Mira eval for `platform` command sequencing, safety, loop budgets,
  and persisted hourly Agent/MCP/model/trigger state.

## 2026-08-06

* **Platform command surface**: Added the high-risk built-in `platform`
  capability with MCP-parity `discover`, read-only `query`, and mutating
  `execute` tools. Platform Chat now uses the shared command inventory, and the
  worker transport re-establishes the session owner's authorization server-side.

* **Main synchronization**: Migrated the newly landed Sans-IO turn-state and WebMCP
  specifications into the OKF bundle and incorporated their feature-flag and threat-model updates.
* **Migration**: Moved the canonical `specs/` corpus into an OKF v0.2 bundle under
  `knowledge/`, preserving the specifications' semantics while adding concept metadata
  and domain indexes.
* **Enforcement**: Added a local conformance and link checker plus the upstream
  `okf-lint` CI gate. Maintenance rules are recorded in the
  [Knowledge Maintenance Contract](knowledge-contract.md).
