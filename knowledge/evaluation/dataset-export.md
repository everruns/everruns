---
type: Specification
title: "Dataset Export Specification"
description: "Reward-labeled trajectory dataset export from eval runs."
tags:
  - everruns
  - evaluation
---
# Dataset Export Specification

## Abstract

Dataset export turns a completed eval run into a **reward-labeled trajectory
dataset**: for each case, "what the model saw → what it did → how good it was",
serialized as newline-delimited JSON (NDJSON). It is the connective tissue
between running evals and improving the models behind them (post-training), and
it immediately improves trace-driven harness iteration (diffing passing vs.
failing trajectories).

This is an ETL/join over data Everruns already persists: reward from
`EvalCaseResult.scores` + status, trajectory content from the eval-tagged
session's events, and efficiency metadata from the case result. It is gated for
privacy because, unlike aggregate reporting, it exports raw model-view content.

## Goals

1. Export a completed `EvalRun` as reward-labeled NDJSON, one record per case.
2. Be faithful to what the model saw, use the **model view** (post-compaction
   masking), not the lossless durable log.
3. Be privacy- and tenant-safe by construction: org-scoped, behind an explicit
   policy, with redaction controls and always-on secret scrubbing.
4. Reuse existing infrastructure (the NDJSON artifact-export precedent, the
   compaction model-view masking, the message reconstruction-from-events path).

## Non-Goals

- Continuous capture from arbitrary production sessions (Phase 1 is eval runs
  only, bounded, consented, already labeled).
- RL-style preference-pair / DPO construction (Phase 1 is SFT-shaped single
  trajectories).
- The post-training / fine-tune orchestration itself (this only produces its
  input).
- Cross-org or platform-wide datasets.

## Source-of-truth join

| Field | Source |
|-------|--------|
| reward (pass/fail, scorer values) | `EvalCaseResult.status` + `EvalCaseResult.scores` |
| trajectory (messages, tool calls/results) | session `events`, reconstructed to `Vec<Message>` then masked to the model view |
| efficiency metadata (turns, tokens, latency) | `EvalCaseResult` |

Reporting facts (`knowledge/evaluation/reporting.md`) deliberately exclude prompts / messages /
tool args / results per `TM-OBS`, so they are the right source for *filtering*,
not *content*. Trajectory content therefore comes from the session events, not
reporting.

### Model-view faithfulness

Trajectories use the compaction **model-view masking**
(`build_model_view_messages` in `crates/builtins/src/compaction.rs`) so
the exported messages match training reality rather than a lossless log the
model never read. Phase 1 applies the default `CompactionConfig`; honoring the
exact per-run compaction config is a follow-up (see Follow-ups).

## API

`POST /v1/evals/{eval_id}/runs/{run_id}/dataset`

Body:

```json
{
  "format": "trajectory" | "sft" | "atif",
  "filters": { "pass": true, "min_score": 0.5 },
  "redaction": { "redact_content": false }
}
```

The run must be `Completed`. The export is **async by handle** (Phase 2): the
`POST` enqueues a background export job (mirroring the fire-and-forget eval
runner) and returns `202 Accepted` with a dataset handle:

```json
{ "id": "evaldataset_…", "eval_run_id": "evalrun_…", "status": "pending", "created_at": "…", "updated_at": "…" }
```

`GET /v1/evals/{eval_id}/runs/{run_id}/dataset/{dataset_id}` returns the handle
with its current `status` (`pending` → `running` → `completed`/`failed`) and,
once `completed`, the produced NDJSON in `body` plus a `record_count` (one record
per surviving case). Both endpoints are gated by `DATASET_EXPORT` and org-scoped
through `get_run`, so a dataset from another org/run is never reachable.

The produced dataset is stored on the handle row, so re-fetching is cheap and the
export is a durable derived artifact (deleting the underlying run clears the FK
via `ON DELETE SET NULL`, matching the "datasets are derived artifacts" rule
below). The sibling artifacts export (`GET …/runs/{run_id}/artifacts`) remains
synchronous.

Identical requests for the same run reuse the existing handle. At most four
dataset exports run concurrently in a server process, and an export fails rather
than storing a body larger than the shared 50 MiB artifact-export limit.

### Output schema

- `trajectory` (generic, canonical):
  `{ source_key, eval_run_id, case_id, case_name, session_id, reward: { pass, score, scorers: [{name, value, pass, reason}] }, messages: [...model-view messages...], metadata: { model, turns, input_tokens, output_tokens, latency_ms } }`
- `sft`: `{ source_key, messages: [{role, content}], reward: { pass, score, scorers } }`, chat-message shape loadable by common SFT pipelines, with reward in a sidecar field so verifiable-reward filtering happens before training.
- `atif`: one complete ATIF-v1.7 trajectory object per line, folded from the
  case session's **model-view messages** (the same post-compaction masking as
  the other dataset formats), with reward and case identity in root `extra`
  (`extra.reward = { pass, score, scorers }`, `extra.source_key`, ...). Because
  it folds the model view rather than the raw event log, ATIF rows carry no
  per-step token metrics or turn roll-up (messages lack per-step usage);
  case-level token totals appear in `final_metrics`. Same policy gate, filters,
  scrubbing, and redaction as the other formats. See `knowledge/evaluation/atif-adoption.md`.
  (The whole-session export, `?format=atif`, still folds the raw event log, it
  is a debug/backup surface, not training data.)

Scorer identities are not persisted on the result (only the ordered
`Vec<Score>`), but the runner emits exactly one score per scorer in definition
order, so each scorer's `name` is joined positionally from the case definition's
`scorers` (the scorer kind, e.g. `contains`, `tool_called`). The join is applied
only when the scorer count matches the score count; otherwise the export falls
back to positional `scorer_0`, … labels rather than risk a mislabel.

### Idempotency

Each record carries a stable `source_key` (`{eval_run_id}/{case_result_id}`) so
re-export is idempotent and records can be deduplicated downstream.

### Selection filters

- `pass`, keep only cases whose pass/fail equals this.
- `min_score`, keep only cases whose mean scorer value is ≥ this.

Filters never reference `org_id`; the org scope is injected from the
authenticated caller and cannot be widened by a filter.

## Privacy / security

- Gated by a dedicated `DATASET_EXPORT` policy (`dataset.export`), distinct from
  `EVAL_VIEW` / `REPORT_VIEW` because this exports raw content. It requires both
  `OrgAgentsManage` and `OrgSessionsManage`, matching the privilege of starting
  runs.
- Org-scoped end-to-end: the run is resolved through `get_run` with the caller's
  org, so cases from other orgs are never reachable.
- **Secret scrubbing is always on**: credential-looking substrings (provider
  keys, AWS keys, GitHub tokens, bearer tokens, `key/secret/password/token`
  assignments) are removed from every exported string.
- **Configurable redaction** (`redact_content`): replaces message text and tool
  content with a placeholder while preserving structure (roles, tool names,
  ids). Off by default.
- Datasets are derived artifacts: deleting the underlying session/eval removes
  the source, so re-export reflects deletions.

See `knowledge/security/threat-model.md` (TM-OBS-008) for the threat review.

## Follow-ups

- Honor the exact per-run compaction config for model-view reconstruction.
  Requires resolving the run's session capability chain
  (harness/agent/session), which the current export path does not plumb; the
  default `CompactionConfig` is used until then.
- Optional cost (`cost_usd`) via reporting-fact join. `fact_llm_generation`
  stores tokens, not a cost column, and is Postgres-only (absent in-memory), so
  a faithful cost join needs a model-pricing lookup rather than a straight join.
- Preference-pair / DPO dataset construction.
- Continuous capture from production sessions (opt-in).
