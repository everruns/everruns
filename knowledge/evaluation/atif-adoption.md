---
type: Specification
title: "ATIF (Agent Trajectory Interchange Format) Adoption"
description: "ATIF (Agent Trajectory Interchange Format) export/import adoption."
tags:
  - everruns
  - evaluation
---
# ATIF (Agent Trajectory Interchange Format) Adoption

## Abstract

[ATIF][atif-rfc] (Harbor RFC 0001) is a vendor-neutral JSON format for agent
trajectories: one root document per trajectory with an `agent` descriptor, a
sequential `steps[]` history (user/system/agent steps with tool calls,
observations, and per-step token metrics), and an `extra` extension point at
every level. For everruns it is an **interchange boundary**, not a new storage
model: the session event log (`knowledge/execution/events.md`) remains the ground truth, and
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
  `knowledge/runtime-resources/session-export.md` and the Limits section; default JSONL unchanged).
  The MCP/CLI `export_session_messages` surface is whole-document only,
  segmentation is an HTTP-route concern and is not exposed to the scripting
  catalog. Reachable from all three consumer surfaces: the API, the UI
  ("Export ATIF"), and the CLI (`everruns sessions export --format atif`,
  default `jsonl`).
- **Dataset export** (✅ shipped): `format: "atif"` on the eval dataset
  export produces NDJSON with one complete ATIF trajectory per case (see
  `knowledge/evaluation/dataset-export.md`). Like the other dataset formats, ATIF folds the
  **model-view messages** (post-compaction masking), so training rows never
  contain content the model did not read; it carries no per-step token metrics
  (messages lack per-step usage). This differs from the whole-session export
  below, which folds the raw event log. Same policy gate, filters, and
  redaction controls.
- **Import** (✅ shipped): `POST /v1/evals/{eval_id}/atif_import` accepts
  NDJSON or JSON (array, single object, or `{ "trajectories": [...] }`) and
  upserts eval cases: user steps → the case `conversation` (multi-turn
  preserved), the final agent message → a reference excerpt in the case
  description. **Import creates unscored cases**: ATIF carries no assertion
  semantics, so no scorer is auto-synthesized from the final message (a
  whole-text `contains` scorer would be brittle); users attach scorers after
  import. Idempotency keys on the case name (derived from `extra.case_name` →
  `extra.source_key` → `trajectory_id` → `session_id`), so re-import
  converges.

## Image content

Image content parts are exported as ATIF multimodal ContentParts (Harbor
RFC 0001 / ATIF v1.6), not dropped. When a step message or a tool-result
observation contains image parts, its `message` / `content` becomes a
**ContentPart array** (text parts + image parts, in order) instead of a
flattened string; text-only content stays a string (the RFC allows either, so
text-only consumers are unaffected). Each image becomes an ATIF image
ContentPart with a `source`, choosing the leanest faithful representation:

- `Image { url }` → `source.path` = the URL.
- `ImageFile { image_id }` → `source.path` = the org-scoped file-serving route
  `/v1/images/{image_id}` (a consumer with the same auth fetches the bytes),
  keeping documents small.
- `Image { base64 }` with no URL → an inline `data:` URI in `source.path`. This
  is the only self-contained option but bloats the document and counts toward
  the export size cap, so it is a last resort.

`source.media_type` is preserved when known. `redact_content` blanks the
content-bearing `source.path` while keeping the structural `media_type`; the
always-on secret scrubber still runs over every produced source.

**Omitted images (now rare).** Only an image that cannot be materialized, an
inline `Image` carrying neither a URL nor base64 bytes, is flattened to an
`"[image]"` marker; its locator (media_type / filename as present, never bytes)
is recorded in step-level `extra.omitted_images[]` and counted in a root
`extra.images_omitted` total. Both keys appear only when at least one image was
genuinely omitted, so this total is typically 0.

## Subagent trajectories

everruns sessions can spawn subagents (`knowledge/runtime-resources/subagents.md`). When the fold
sees a `spawn_agent` tool result carrying a `subagent_id` (the child session
id), it attaches a `subagent_trajectory_ref` (Harbor RFC 0001) to that
`observation.results[]` entry: `trajectory_path` points at the child's own ATIF
export (`/v1/sessions/{child}/export?format=atif`, a resolvable location per the
RFC's ref rules), plus the informational `session_id`. This is **ref-only**,
the child trajectory is not embedded as `subagent_trajectories[]`, because this
fold sees only one session's event log and has no access to child-session
events. Embedding (recursive fold of resolvable child sessions) is a follow-up.

## Limits
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
    final/only segment omits it, that is how a reader detects the end.
  - The **cursor** is opaque (`base64url(JSON)` of the session id + next step
    offset). It is validated on every request: malformed, foreign-session, or
    out-of-range cursors are rejected with HTTP 400 (code `atif_cursor_invalid`),
    never a panic. The session is always resolved org-scoped from the path; the
    cursor only selects a step offset within that session, so it cannot widen
    scope.
  - Root `extra` carries per-segment bookkeeping: `segment_index`,
    `continued_trajectory_ref` (mirrored), and `images_omitted` (genuinely
    unmaterializable images) for **this** segment. The session-level `turns`
    roll-up is carried once, on the final segment.
  - **Byte bounding.** Segments are packed greedily and stop before the
    serialized segment would exceed `ATIF_EXPORT_MAX_BYTES`; each segment holds
    at least one step. **Caveat:** a single step whose own serialization exceeds
    the cap is returned alone and may exceed the cap, the only way to make
    progress without dropping data (no 413, no infinite loop).
  - Secret scrubbing is applied to **every** segment, same as the whole-doc
    path. Response headers `X-Atif-Segment-Index`, `X-Atif-Images-Omitted` (when
    N > 0, per segment), and `X-Atif-Next-Cursor` (when more remain) let a client
    walk the chain and detect lossiness without parsing each body.
- **Lossiness header.** Successful ATIF session exports set
  `X-Atif-Images-Omitted: <N>` only when N > 0 (per segment on the segmented
  path), so clients can detect a lossy export without parsing the body. Since
  most images now export as content parts, N is typically 0 and the header is
  usually absent.

## Security

- Secret scrubbing (the dataset-export scrubber) is always on for every
  produced ATIF document, on every export surface; `redact_content` blanks
  message/reasoning/argument/observation content while preserving structure.
- Import follows the OKF importer posture (`knowledge/runtime-resources/okf-adoption.md`):
  org-scoped through the eval lookup, gated by `EVAL_MANAGE`, body capped
  (4 MiB, 200 trajectories, 64 KiB per message), malformed input rejected with
  400, no cross-org existence leaks.

## Delivery Plan

Each item is an independently reviewable change (committed PR-sized). Status
reflects what has shipped on `main`.

1. ✅ **Serializer + session export**: `crates/server/src/atif.rs` fold,
   `?format=atif` on session export, this spec, index/cross-link updates.
2. ✅ **Dataset export + import**: `format: "atif"` on the eval dataset
   export, `POST /v1/evals/{eval_id}/atif_import`, and the related spec
   updates (`knowledge/evaluation/dataset-export.md`, `knowledge/evaluation/evals.md`).
3. ✅ **Segmented session export**: `?format=atif&segmented=true` with
   `continued_trajectory_ref` linking + opaque cursor, for sessions over the
   size cap (see Limits).
4. ✅ **Fold fidelity**: image content parts exported as ATIF multimodal
   ContentParts (see Image content) and subagent spawns linked via
   `subagent_trajectory_ref` (see Subagent trajectories).

## Non-goals (v1)

- Embedding subagents as `subagent_trajectories[]` (child sessions are linked
  by ref only; recursive embedding is a follow-up, see Subagent trajectories).
- Importing trajectories as *results* (the existing external-run import in
  `knowledge/evaluation/evals.md` covers scored results; ATIF import produces cases).

[atif-rfc]: https://github.com/harbor-framework/harbor/blob/main/rfcs/0001-trajectory-format.md
