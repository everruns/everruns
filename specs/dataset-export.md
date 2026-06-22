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
2. Be faithful to what the model saw — use the **model view** (post-compaction
   masking), not the lossless durable log.
3. Be privacy- and tenant-safe by construction: org-scoped, behind an explicit
   policy, with redaction controls and always-on secret scrubbing.
4. Reuse existing infrastructure (the NDJSON artifact-export precedent, the
   compaction model-view masking, the message reconstruction-from-events path).

## Non-Goals

- Continuous capture from arbitrary production sessions (Phase 1 is eval runs
  only — bounded, consented, already labeled).
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

Reporting facts (`specs/reporting.md`) deliberately exclude prompts / messages /
tool args / results per `TM-OBS`, so they are the right source for *filtering*,
not *content*. Trajectory content therefore comes from the session events, not
reporting.

### Model-view faithfulness

Trajectories use the compaction **model-view masking**
(`build_model_view_messages` in `crates/core/src/capabilities/compaction.rs`) so
the exported messages match training reality rather than a lossless log the
model never read. Phase 1 applies the default `CompactionConfig`; honoring the
exact per-run compaction config is a follow-up (see Follow-ups).

## API

`POST /v1/evals/{eval_id}/runs/{run_id}/dataset`

Body:

```json
{
  "format": "trajectory" | "sft",
  "filters": { "pass": true, "min_score": 0.5 },
  "redaction": { "redact_content": false }
}
```

Returns `application/x-ndjson` — one record per surviving case. The run must be
`Completed`. This mirrors the synchronous artifacts export
(`GET …/runs/{run_id}/artifacts`). An async dataset-handle + status API
(`POST` enqueues, `GET …/dataset/{dataset_id}`) is a deferred follow-up; Phase 1
returns the NDJSON inline.

### Output schema

- `trajectory` (generic, canonical):
  `{ source_key, eval_run_id, case_id, case_name, session_id, reward: { pass, score, scorers: [{name, value, pass, reason}] }, messages: [...model-view messages...], metadata: { model, turns, input_tokens, output_tokens, latency_ms } }`
- `sft`: `{ source_key, messages: [{role, content}], reward: { pass, score, scorers } }` — chat-message shape loadable by common SFT pipelines, with reward in a sidecar field so verifiable-reward filtering happens before training.

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

- `pass` — keep only cases whose pass/fail equals this.
- `min_score` — keep only cases whose mean scorer value is ≥ this.

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

See `specs/threat-model.md` (TM-OBS-008) for the threat review.

## Follow-ups

- Async dataset-handle API with status + stored dataset artifacts.
- Honor the exact per-run compaction config for model-view reconstruction.
- Optional cost (`cost_usd`) via reporting-fact join.
- Preference-pair / DPO dataset construction.
- Continuous capture from production sessions (opt-in).
