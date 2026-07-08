# ATIF (Agent Trajectory Interchange Format) Adoption

## Abstract

[ATIF][atif-rfc] (Harbor RFC 0001) is a vendor-neutral JSON format for agent
trajectories: one root document per trajectory with an `agent` descriptor, a
sequential `steps[]` history (user/system/agent steps with tool calls,
observations, and per-step token metrics), and an `extra` extension point at
every level. For everruns it is an **interchange boundary**, not a new storage
model: the session event log (`specs/events.md`) remains the ground truth, and
ATIF documents are folded from it on export.

Adopting ATIF lets reward-labeled everruns trajectories feed external
post-training/analysis pipelines, and lets trajectories produced by other
agents (e.g. yolop, Harbor-based harnesses) seed everruns eval cases.

Adoption ships in independently reviewable PRs (see Delivery Plan); this spec
captures the full design intent, with per-surface status noted below.

## Version and tolerance

- Exports pin `schema_version: "ATIF-v1.7"`.
- Imports tolerate any `ATIF-*` version and ignore unknown fields at every
  level (the format's own compatibility rule); only `schema_version`, a
  non-empty `steps[]`, and at least one user step are required.

## Event → ATIF mapping

The fold lives in `crates/server/src/atif.rs` (one implementation shared by
every export surface). Field-level detail is readable there; the mapping:

| everruns events | ATIF |
|---|---|
| `input.message` | one `user` step |
| `output.message.completed` (one per reasoning iteration) | one `agent` step: text → `message`, tool-call parts → `tool_calls[]`, thinking → `reasoning_content`, usage → `metrics` |
| `reason.completed` | annotates the iteration's step (duration, fallback usage); failures become an agent step with `extra.error` |
| `tool.completed` | `observation.results[]` on the iteration's step (`source_call_id` = tool_call_id) |
| `turn.*` | boundaries, not steps; per-turn iterations/duration/status recorded in root `extra.turns` |
| aggregate usage | `final_metrics` |

Everything else in the event log (deltas, lifecycle, budget, voice, ...) has no
ATIF equivalent and is dropped.

## `extra.reward` convention

ATIF has no reward field. Dataset-export records carry the eval reward at the
root extension point: `extra.reward = { pass, score, scorers }` (same shape as
the existing dataset formats), plus case identity (`source_key`, `eval_run_id`,
`case_id`, `case_name`) for idempotent downstream joins.

## Surfaces

- **Session export** (✅ shipped): `GET /v1/sessions/{id}/export?format=atif`
  returns one ATIF JSON document (see `specs/session-export.md`; default JSONL
  unchanged).
- **Dataset export** (✅ shipped): `format: "atif"` on the eval dataset
  export produces NDJSON with one complete ATIF trajectory per case (see
  `specs/dataset-export.md`). Unlike the message-based formats, ATIF folds the
  raw event log (per-iteration steps and observations), not the compaction
  model view. Same policy gate, filters, and redaction controls.
- **Import** (✅ shipped): `POST /v1/evals/{eval_id}/atif_import` accepts
  NDJSON or JSON (array, single object, or `{ "trajectories": [...] }`) and
  upserts eval cases: user steps → the case `conversation` (multi-turn
  preserved), the final agent message → a reference excerpt in the case
  description. **Import creates unscored cases** — ATIF carries no assertion
  semantics, so no scorer is auto-synthesized from the final message (a
  whole-text `contains` scorer would be brittle); users attach scorers after
  import. Idempotency keys on the case name (derived from `extra.case_name` →
  `extra.source_key` → `trajectory_id` → `session_id`), so re-import
  converges.

## Security

- Secret scrubbing (the dataset-export scrubber) is always on for every
  produced ATIF document, on every export surface; `redact_content` blanks
  message/reasoning/argument/observation content while preserving structure.
- Import follows the OKF importer posture (`specs/okf-adoption.md`):
  org-scoped through the eval lookup, gated by `EVAL_MANAGE`, body capped
  (4 MiB, 200 trajectories, 64 KiB per message), malformed input rejected with
  400, no cross-org existence leaks.

## Delivery Plan

Each item is an independently reviewable change (committed PR-sized). Status
reflects what has shipped on `main`.

1. ✅ **Serializer + session export** — `crates/server/src/atif.rs` fold,
   `?format=atif` on session export, this spec, index/cross-link updates.
2. ✅ **Dataset export + import** — `format: "atif"` on the eval dataset
   export, `POST /v1/evals/{eval_id}/atif_import`, and the related spec
   updates (`specs/dataset-export.md`, `specs/evals.md`).

## Non-goals (v1)

- `subagent_trajectories` (session tasks are not folded into embedded
  subagent trajectories yet).
- ATIF image content parts (images are flattened to markers).
- Importing trajectories as *results* (the existing external-run import in
  `specs/evals.md` covers scored results; ATIF import produces cases).

[atif-rfc]: https://github.com/harbor-framework/harbor/blob/main/rfcs/0001-trajectory-format.md
