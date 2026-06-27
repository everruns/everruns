# Proposal: Everruns as a host for Mira eval results

Status: draft proposal (pre-spec). Spans three repos: `everruns` (primary),
`mira`, `saas`. On acceptance this splits into specs + Linear issues per
workstream.

## Problem

Everruns has an unreleased **evals** system (flag `evals`). It is
*execution-coupled*: an `EvalRun` only exists because everruns itself spawned a
real session per case, drove the conversation, and scored it with nine built-in
scorer rules. The only external write path is score *write-back*
(`PATCH .../scores`) onto results everruns already executed. There is **no way
to import a run everruns did not run.**

Mira owns execution + scoring end to end. It runs any subject (Anthropic,
OpenAI, a CLI process, or the everruns runtime), produces a rich `Transcript`
(tokens, cost, time-to-first-token, tool calls, open-vocab metrics, multimodal
parts, raw event JSONL, files) and renders good *local* reports — but has **no
external sink.** (`mira-everruns` is consumptive: Mira drives the everruns
runtime *as a subject*. It is not a publishing channel.)

So three things are true at once:

1. People who want hosted, shareable, comparable eval results have to run inside
   everruns' session system — friction that is bad for onboarding external
   parties.
2. Mira can produce those results but cannot publish them anywhere.
3. Everruns' eval UI cannot yet *compare* runs even when it has them
   (Phase-1 only: single-run table; no matrix, deltas, trends, or transcript
   drill-down).

## Goal

Make everruns a capable **host** for eval results — including results it did not
execute — and give Mira a first-class way to publish to it. Concretely:

- **(a)** Everruns ingests a complete, externally-executed eval run and stores
  it as first-class eval data.
- **(b)** Everruns' eval UI can compare and visualize runs (matrix, deltas,
  regressions, trends, per-case transcript drill-down) — source-agnostic.
- **(c)** Mira publishes a finished run to everruns with one command / CI step.

### Non-goals

- Re-running or re-scoring Mira results inside everruns. External verdicts are
  trusted and stored as-is.
- Changing the Mira ⇄ subject protocol. Publishing is a host-side concern, not a
  protocol change.
- Mapping Mira's open-vocab scorers onto everruns' nine-variant scorer enum.
- A new standalone "results host" entity. We extend the existing eval model
  (decision below).

## Core decision: decouple "results as data" from "execution as orchestration"

Today the only way to get a run into everruns is to make everruns run it. We
split those concerns by extending the existing model with an external source,
rather than introducing a parallel entity. (Considered and rejected: a separate
lightweight imported-results entity — cleaner separation, but a second data
model and a second UI to maintain forever, for a feature whose whole value is
appearing *next to* native runs in the same comparison views.)

The existing model already carries most of what we need:

- `EvalCaseResult` holds **per-result** `target` + `target_snapshot`, so one
  `EvalRun` can already represent a full *case × target* matrix within an eval.
- Results allow `session_id` to be **optional** already — external results have
  no everruns session.
- The external score write-back path proves arbitrary `{scorer, value, pass,
  reason}` scores round-trip through the `scores` JSONB.

## Mapping Mira → everruns

A Mira *run* spans multiple evals × targets; an everruns `EvalRun` is scoped to
one `Eval`. So:

- Mira **eval** → everruns **`Eval`** (upsert by slug).
- Mira **run** → a **run-group** of everruns `EvalRun`s (one per Mira eval),
  tied by a shared `source_run_id` = Mira's stable, sortable run id.
- Within each `EvalRun`, all targets live across `EvalCaseResult`s via
  per-result `target_snapshot`. Cross-eval matrix = query EvalRuns by
  `source_run_id`. No new entity — just new columns on `eval_runs`.
- Mira **sample** → everruns **`EvalCase`** (upsert by sample key within eval).
- Mira **`RunResult`** → everruns **`EvalCaseResult`**.

| Mira | Everruns | Notes |
|---|---|---|
| `eval` (name) | `Eval` (slug) | upsert; auto-provision on publish |
| run `run_id` | `EvalRun.source_run_id` | group key + idempotency key |
| `sample` (key) | `EvalCase` (key) | identity-only for external; no conversation/scorers reconstructed |
| `target {provider, model}` + `params` | `EvalCaseResult.target_snapshot` | new label-only target variant (below) |
| `RunResult.scores[] {scorer, value, pass, na, reason}` | result `scores` JSONB | opaque, names preserved; `na` kept in payload |
| `passed` / `skipped` | result `status` | add `skipped` status to preserve semantics |
| `Transcript.final_response` | result transcript blob | stored, rendered natively |
| `Transcript.events[]` / `files{}` / `output[]` parts | result transcript blob / artifacts | structured store, not a synthesized session |
| `usage {input, output, cache_read, reasoning, cost_usd}` | `input_tokens` / `output_tokens` + `metadata` | extras (cost, cache, reasoning) in metadata |
| `timing {duration_ms, ttft_ms}` | `latency_ms` + `metadata` | ttft in metadata |
| `iterations` / `tool_calls_count` | `turns` + `metadata` | |
| `metrics {}` (open vocab) | `metadata` | recall@k, p95, energy, etc. |
| `RunSummary` (per eval × target) | `RunSummary` | scored/passed/failed map; na/skipped/cost/tool_calls as extras |
| run `Environment` (git, os, mira version, labels) | `EvalRun.metadata` | provenance |

## Workstream (a): everruns ingest backend

**Data model**

- `eval_runs`: add `source` (`internal` | `external`), `executed_by`
  (`"mira"` + version), `source_run_id` (group key, nullable for internal).
  External runs are born `completed`; they never spawn sessions.
- `EvalTarget`: add a label-only variant
  `External { provider, model, params }` (the existing variants are
  `Session{...}` / `App{app_id}`, both session-setup contracts that do not fit
  an externally-executed run).
- `EvalCaseResult.status`: add `skipped` to preserve Mira's "case never
  executed" semantics. Fold cost / cache / reasoning tokens / ttft / iterations
  / open-vocab metrics into the existing `metadata` JSONB.
- **Transcript storage**: store Mira's transcript (final response + event JSONL
  + files + multimodal parts) as a structured blob attached to the result.
  *Decision:* store-and-render natively. Rejected alternatives — link out to
  Mira's HTML report (defeats the point of hosting) or synthesize a fake
  read-only everruns session (event schemas differ; brittle).

**API**

- `POST /v1/evals:import` (run-group ingest): one call accepts a whole Mira run
  — run meta, and per eval the cases + results + scores + transcripts. Server
  upserts evals/cases by slug, creates the `EvalRun`(s) under one
  `source_run_id`, writes results. **Idempotent** on `source_run_id`
  (re-publish replaces the group).
- Auto-provisioning: publishing creates the `Eval`/`EvalCase` rows if absent so
  external authors never pre-register in everruns. (Open question OQ-1 — could
  be gated behind explicit registration if we want less magic.)

**Auth & permissions**

- Reuse **Personal Access Tokens** — already a supported `Bearer` method in all
  auth modes including `external` (PropelAuth / SaaS). No new auth subsystem; CI
  publishes with a PAT.
- New policy `eval.import`, gated by `OrgAgentsManage` only. Unlike `eval.run`
  it does **not** require `OrgSessionsManage` (no sessions are created).
- Org-scoped, fail-closed, feature-flagged under `evals` like the rest.

## Workstream (b): everruns eval UI — comparison & visualization

The run-detail page branches on `source`: external runs swap "open session"
links for the transcript drill-down. Everything else is **source-agnostic** —
the new views read results + summaries identically whether everruns or Mira
produced them. New surfaces (the Phase-2 gap):

- **Multi-run comparison**: pick N runs → per-case table with score deltas and
  **regression highlighting** (passed → failed).
- **Model matrix**: target × eval grid (Mira's signature view), reading a
  `source_run_id` group.
- **Per-scorer aggregation**, and **trends** (pass-rate / cost / latency across
  runs over time).
- **Per-case transcript drill-down** rendering Mira events + multimodal parts.
- Filtering / search / export.

## Workstream (c): Mira publish sink

- New integration crate `mira-publish-everruns` (keeps `mira-eval` core
  provider-agnostic, same boundary as `mira-everruns`).
- CLI: `mira publish <run_dir> --to everruns` and `mira run --publish everruns`.
- Config in `mira.toml`: everruns base URL, PAT (from env), eval slug / project.
- Maps `RunMeta` + `RunResult[]` (already persisted in the run folder) to the
  import payload. Idempotent on Mira's `run_id`.
- **Decoupled from `mira-everruns`**: any provider or CLI subject can publish.
  You do *not* have to run inside the everruns runtime to host results in
  everruns — this is the onboarding win.

## Optional: external-party viewing (SaaS)

For stakeholders without an org seat: token-based **shareable read-only
eval-results pages** in SaaS, reusing the isolated support-app pattern
(separate surface, non-bearer share token, no PropelAuth). Lets external parties
view comparisons without being onboarded into the run system. Deferrable past v1.

## Phasing

1. **Backend ingest** (a): model columns, `External` target, `:import`
   endpoint, `eval.import` policy + PAT, transcript storage. Unblocks Mira.
2. **Mira sink** (c): `mira-publish-everruns` + CLI. End-to-end publish works;
   results visible in the existing single-run UI.
3. **Comparison UI** (b): matrix, deltas, regressions, trends, drill-down.
4. **SaaS sharing** (optional): shareable read-only pages.

(1) and (2) are the minimum for "Mira can publish and you can see it." (3) is
the bulk of the user-facing value. (4) is additive.

## Open questions

- **OQ-1 Auto-provisioning vs. explicit registration.** Publish silently
  upserts evals/cases by slug (low friction) vs. requiring the eval to be
  registered in everruns first (safer, more friction). Proposal assumes
  auto-provision; flagged for confirmation.
- **OQ-2 Transcript fidelity for v1.** Full native render of Mira's event
  stream (more UI work) vs. "summary + scores in everruns, full transcript via
  link to Mira's HTML" as a v1 shortcut. Proposal assumes native render as the
  end state; v1 could ship the shortcut.
- **OQ-3 Slug collisions / ownership.** How an org namespaces eval slugs coming
  from multiple Mira projects (prefix? project field on import?).
