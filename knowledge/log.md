# Everruns Knowledge Update Log

## 2026-08-06

* **Main synchronization**: Migrated the newly landed Sans-IO turn-state and WebMCP
  specifications into the OKF bundle and incorporated their feature-flag and threat-model updates.
* **Migration**: Moved the canonical `specs/` corpus into an OKF v0.2 bundle under
  `knowledge/`, preserving the specifications' semantics while adding concept metadata
  and domain indexes.
* **Enforcement**: Added a local conformance and link checker plus the upstream
  `okf-lint` CI gate. Maintenance rules are recorded in the
  [Knowledge Maintenance Contract](knowledge-contract.md).
