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
  returns one ATIF JSON document; `&segmented=true` returns a forward-linked
  chain of byte-bounded segments for sessions over the size cap (see
  `specs/session-export.md` and the Limits section; default JSONL unchanged).
  The MCP/CLI `export_session_messages` surface is whole-document only —
  segmentation is an HTTP-route concern and is not exposed to the scripting
  catalog. Reachable from all three consumer surfaces: the API, the UI
  ("Export ATIF"), and the CLI (`everruns sessions export --format atif`,
  default `jsonl`).
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

## Limits

- **Images are never exported raw.** Every image content part is flattened to
  an `"[image]"` marker in step/observation text, and the fold records what
  was omitted: locators only (url / file_id / media_type / filename as
  present on the part, never bytes) in step-level `extra.omitted_images[]`,
  plus a root `extra.images_omitted` total. Both keys appear only when at
  least one image was omitted.
- **Session export size cap.** Plain `?format=atif` returns one synchronous
  JSON document and enforces `ATIF_EXPORT_MAX_BYTES` (50 MiB, defined in
  `crates/server/src/atif.rs`); larger documents are rejected with HTTP 413
  (standard error JSON, code `atif_export_too_large`, with `detail` pointing at
  `&segmented=true`). The default JSONL export is not affected.
- **Segmented export (recoverable path for large sessions).**
  `?format=atif&segmented=true` returns the session as a forward-linked chain of
  byte-bounded segments instead of one document, so a session over the cap stays
  exportable. Contract:
  - Each segment is a **standalone, valid ATIF-v1.7 document** with the same
    `schema_version` and `session_id`, a per-segment `trajectory_id`
    (`{session_id}#segment-{index}`), a prefix window of the session's `steps[]`
    with **absolute `step_id`s preserved**, and per-segment `final_metrics`
    (summed over that segment's steps). Concatenating every segment's `steps[]`
    in order reproduces the whole-document `steps[]`.
  - A segment with more steps remaining carries the RFC's
    **`continued_trajectory_ref`** (a root string, per Harbor RFC 0001): the
    export URL for the next segment, embedding an opaque `cursor`. The
    final/only segment omits it — that is how a reader detects the end.
  - The **cursor** is opaque (`base64url(JSON)` of the session id + next step
    offset). It is validated on every request: malformed, foreign-session, or
    out-of-range cursors are rejected with HTTP 400 (code `atif_cursor_invalid`),
    never a panic. The session is always resolved org-scoped from the path; the
    cursor only selects a step offset within that session, so it cannot widen
    scope.
  - Root `extra` carries per-segment bookkeeping: `segment_index`,
    `continued_trajectory_ref` (mirrored), and `images_omitted` for **this**
    segment. The session-level `turns` roll-up is carried once, on the final
    segment.
  - **Byte bounding.** Segments are packed greedily and stop before the
    serialized segment would exceed `ATIF_EXPORT_MAX_BYTES`; each segment holds
    at least one step. **Caveat:** a single step whose own serialization exceeds
    the cap is returned alone and may exceed the cap — the only way to make
    progress without dropping data (no 413, no infinite loop).
  - Secret scrubbing is applied to **every** segment, same as the whole-doc
    path. Response headers `X-Atif-Segment-Index`, `X-Atif-Images-Omitted` (when
    N > 0, per segment), and `X-Atif-Next-Cursor` (when more remain) let a client
    walk the chain and detect lossiness without parsing each body.
- **Lossiness header.** Successful ATIF session exports set
  `X-Atif-Images-Omitted: <N>` only when N > 0 (per segment on the segmented
  path), so clients can detect a lossy export without parsing the body.

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
3. ✅ **Segmented session export** — `?format=atif&segmented=true` with
   `continued_trajectory_ref` linking + opaque cursor, for sessions over the
   size cap (see Limits).

## Non-goals (v1)

- `subagent_trajectories` (session tasks are not folded into embedded
  subagent trajectories yet).
- ATIF image content parts (images are flattened to markers with locator
  records — see Limits).
- Importing trajectories as *results* (the existing external-run import in
  `specs/evals.md` covers scored results; ATIF import produces cases).

[atif-rfc]: https://github.com/harbor-framework/harbor/blob/main/rfcs/0001-trajectory-format.md
