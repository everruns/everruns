---
type: Specification
title: "External evaluation results publishing"
description: "Contract and remaining design work for publishing externally executed evaluation results into Everruns."
tags:
  - everruns
  - evaluation
  - reporting
  - mira
---

# External evaluation results publishing

Status: partially implemented. Everruns external import/preflight/attribution,
transcript, matrix, comparison, regression, and sharing surfaces are shipped;
Mira also has an Everruns publishing sink. The remaining proposal scope is
per-scorer aggregation and longitudinal trend views, together with a decision
on the filtering/export UX described below. The shipped contract is owned by
[`evals.md`](evals.md); the
problem statement below records the pre-implementation baseline.

## Problem

Everruns has an unreleased **evals** system (flag `evals`, not GA). It is
*execution-coupled*: an `EvalRun` only exists because everruns itself spawned a
real session per case, drove the conversation, and scored it with nine built-in
scorer rules (a **closed enum**). The only external write path is score
*write-back* onto results everruns already executed. There is **no way to
import a run everruns did not run.**

Mira owns execution + scoring end to end. It runs any subject (Anthropic,
OpenAI, a CLI process, or the everruns runtime), produces a rich `Transcript`
(tokens, cost, time-to-first-token, tool calls, open-vocab metrics, multimodal
parts, raw event JSONL, files) and renders good *local* reports, but has **no
external sink.** (`mira-everruns` is consumptive: Mira drives the everruns
runtime *as a subject*. It is not a publishing channel.)

So three things are true at once:

1. People who want hosted, shareable, comparable eval results have to run inside
   everruns' session system, friction that is bad for onboarding external
   parties.
2. Mira can produce those results but cannot publish them anywhere.
3. Everruns' eval UI cannot yet *compare* runs even when it has them
   (Phase-1 only: single-run table; no matrix, deltas, trends, or transcript
   drill-down).

## Goal

**Everruns becomes a vendor-neutral host and viewer for external eval systems.**
Mira is the first client, but the import API, scoring model, transcript view,
and attribution are designed for *any* external eval source, not Mira
specifically. Concretely:

- **(a)** Everruns ingests a complete, externally-executed eval run and stores
  it as first-class eval data, with **attribution** to its source.
- **(b)** Everruns' eval UI compares and visualizes runs (matrix, deltas,
  regressions, trends) and renders a **generic, provider-agnostic transcript
  view**: source-agnostic throughout.
- **(c)** Mira publishes a finished run to everruns with one command / CI step,
  **reusing everruns CLI auth**, after a **permission preflight**.

### Non-goals

- Re-running or re-scoring external results inside everruns. **External verdicts
  are trusted and stored as-is**: everruns is a viewer, not a re-grader.
- Changing the Mira ⇄ subject protocol. Publishing is a host-side concern.
- A new standalone "results host" entity. We extend the existing eval model.
  Because evals are **not GA**, we are free to restructure existing shapes
  (naming, columns → extensible structures) rather than only adding alongside.

## Core decisions (resolved)

These were the open questions; here is where they land.

1. **Extend `EvalRun` with `source = external`**, not a parallel entity.
   Rejected: a second model + second UI forever, for a feature whose value is
   appearing *next to* native runs in the same comparison views.

2. **Vendor-neutral, not Mira-specific.** Everything below is keyed on a generic
   `source` (system name + version + link), so a future external system reuses
   the same path. Mira is the reference implementation.

3. **Auto-provision is OK**, but **gated behind a permission preflight.**
   Publishing silently upserts evals/cases by slug, *after* the client
   confirms the user has `eval.import` and the `evals` feature is enabled.
   Evals are optional, so the client must degrade gracefully when they are off.

4. **Scoring becomes extensible, not a closed enum.** Everruns' nine-variant
   scorer enum becomes an open model where a score is an attributed,
   named entry (`{scorer, value, pass, reason, source}`), the built-in rules
   are one source; external systems are another. Decouples *scorer-as-rule*
   (something everruns can execute) from *score-as-data* (a result everruns
   stores and displays).

5. **Generic transcript view.** Build a provider-agnostic transcript viewer in
   the everruns UI that renders a normalized transcript schema (messages, tool
   calls, events, multimodal parts, files). External transcripts normalize into
   it; native everruns sessions can render through it too. Not a Mira-only
   widget, not a link-out to Mira's HTML.

6. **Stop modeling per-result signal as columns.** cost, cache/reasoning tokens,
   ttft, tool calls, iterations, latency, even token counts, move to an
   **extensible metrics bag** (open-vocab `metrics`/`metadata`) instead of
   accreting columns that never keep up. New signal is just a new key.

7. **Attribution is first-class.** External evals/runs/results must *look
   natural* in everruns while clearly carrying provenance: which external system
   produced them, version, and a link back. Surfaced as an attribution
   badge/field on evals, runs, and results, vendor-neutral.

## Mapping external run → everruns (Mira as reference)

A Mira *run* spans multiple evals × targets; an everruns `EvalRun` is scoped to
one `Eval`. So:

- external **eval** → everruns **`Eval`** (upsert by slug).
- external **run** → a **run-group** of everruns `EvalRun`s (one per eval),
  tied by a shared `source_run_id` = the external system's stable run id.
- Within each `EvalRun`, all targets live across `EvalCaseResult`s via
  per-result `target_snapshot`. Cross-eval matrix = query EvalRuns by
  `source_run_id`. No new entity, new columns on `eval_runs`.
- external **sample/case** → everruns **`EvalCase`** (upsert by key within eval;
  identity-only, name + key + optional display input; conversation/scorers
  empty, since everruns never re-runs it).

| Mira | Everruns | Notes |
|---|---|---|
| `eval` (name) | `Eval` (slug) | upsert; auto-provision after preflight |
| run `run_id` | `EvalRun.source_run_id` | group key + idempotency key |
| run `Environment` + study/version | `EvalRun` attribution + `metadata` | source = `mira` + version, git, labels |
| `sample` (key) | `EvalCase` (key) | identity-only |
| `target {provider, model}` + `params` | `EvalCaseResult.target_snapshot` | new label-only target variant |
| `scores[] {scorer, value, pass, na, reason}` | result scores (extensible) | named + attributed; `na` preserved |
| `passed` / `skipped` | result `status` | add `skipped` status |
| `Transcript` (final, events, files, parts) | normalized transcript blob | rendered by the generic transcript view |
| `usage` / `timing` / `iterations` / `tool_calls` / `metrics{}` | result **metrics bag** | open-vocab; cost, cache, reasoning, ttft, p95, recall@k, … |
| `RunSummary` (per eval × target) | `RunSummary` | grouped; extras live in the metrics bag |

## Workstream (a): everruns ingest backend

**Data model** (evals not GA → restructure freely)

- `eval_runs`: add `source` (`internal` | `external`), source attribution
  (`system` name, `version`, optional `url`), `source_run_id` (group key).
  External runs are born `completed`; they never spawn sessions.
- `EvalTarget`: add a label-only variant `External { provider, model, params }`
  (existing `Session{...}` / `App{app_id}` variants are session-setup contracts
  that do not fit an externally-executed run).
- **Scoring model → extensible.** Replace the closed scorer enum's role as the
  *only* score shape with an open, attributed score record. Built-in rules
  remain as one `source`; external/named scorers are first-class. (See
  decision 4.)
- **Per-result signal → metrics bag.** Migrate cost / tokens / cache / reasoning
  / ttft / turns / latency / iterations / open-vocab metrics into an extensible
  `metrics` structure rather than columns. (See decision 6.)
- `EvalCaseResult.status`: add `skipped`.
- **Normalized transcript** stored per result (messages, tool calls, events,
  multimodal parts, files), the schema the generic view reads.
- **Attribution** fields on `Eval` / `EvalRun` / `EvalCaseResult`.

**API**

- `POST /v1/evals/import` (run-group ingest): one call accepts a whole external
  run, run meta + attribution, and per eval the cases + results + scores +
  normalized transcripts. Server upserts evals/cases by slug, creates the
  `EvalRun`(s) under one `source_run_id`, writes results. **Idempotent** on
  `source_run_id` (re-publish replaces the group).
- **Preflight**: a capability/permission check the client calls *first*,
  reports whether `evals` is enabled and whether the caller holds `eval.import`.
  Lets optional-feature clients degrade gracefully. (Can reuse / extend an
  existing capabilities or `GET /v1/auth/me` style endpoint.)

**Auth & permissions**

- Reuse **Personal Access Tokens** (`evr_pat_…`, sent `Authorization: Bearer`),
  already supported in all auth modes including `external`/PropelAuth (SaaS).
  No new auth subsystem.
- New policy `eval.import`, gated by `OrgAgentsManage` only (no
  `OrgSessionsManage`, no sessions created). Org-scoped, fail-closed,
  feature-flagged under `evals`.

## Workstream (b): everruns eval UI, comparison + generic transcript

The run-detail page is **source-agnostic**: it reads results + summaries + the
normalized transcript identically whether everruns or an external system
produced them. External runs carry an attribution badge. New surfaces:

- **Generic transcript view**: provider-agnostic component rendering the
  normalized transcript (messages, tool calls, events, multimodal parts, files).
  Reusable across native sessions and external runs.
- **Multi-run comparison**: pick N runs → per-case table with score deltas and
  **regression highlighting** (passed → failed).
- **Model matrix**: target × eval grid (reads a `source_run_id` group).
- **Per-scorer aggregation** (works because scores are named/attributed), and
  **trends** (pass-rate / cost / latency across runs over time).
- Filtering / search / export.

## Workstream (c): Mira publish sink

- New integration crate `mira-publish-everruns` (keeps `mira-eval` core
  provider-agnostic, same boundary as `mira-everruns`).
- CLI: `mira publish <run_dir> --to everruns` and `mira run --publish everruns`.
- **CLI auth pass-through**: reuse everruns CLI credentials. Resolution order
  mirrors everruns' own CLI, `--api-key` flag > `EVERRUNS_API_KEY` env >
  everruns credentials file (`~/.config/everruns/credentials.json`, multi-profile,
  `--profile`), `EVERRUNS_API_URL` for the base URL. If you've run
  `everruns login`, `mira publish` just works.
- **Preflight before publish**: call the capability check; if `evals` is off or
  the user lacks `eval.import`, fail clearly (this is an optional feature, not an
  error in the eval run itself).
- Maps `RunMeta` + `RunResult[]` (already persisted in the run folder) to the
  import payload; normalizes the Mira `Transcript` into everruns' transcript
  schema. Idempotent on Mira's `run_id`.
- **Decoupled from `mira-everruns`**: any provider or CLI subject can publish.
  You do *not* have to run inside the everruns runtime to host results in
  everruns, this is the onboarding win.

## Optional: external-party viewing (SaaS)

For stakeholders without an org seat: token-based **shareable read-only
eval-results pages** in SaaS, reusing the isolated support-app pattern
(separate surface, non-bearer share token, no PropelAuth). Deferrable past v1.

## Phasing

1. **Backend ingest** (a): `source`/attribution + `source_run_id` columns,
   `External` target, extensible scoring + metrics bag, normalized transcript
   storage, `/v1/evals/import`, preflight, `eval.import` policy + PAT. Unblocks
   Mira.
2. **Mira sink** (c): `mira-publish-everruns` + CLI + auth pass-through +
   preflight. End-to-end publish; results visible in the existing single-run UI.
3. **UI** (b): generic transcript view, then matrix / comparison / deltas /
   regressions / trends.
4. **SaaS sharing** (optional): shareable read-only pages.

(1)+(2) are the minimum for "Mira can publish and you can see it." (3) is the
bulk of the user-facing value. (4) is additive.

## Resolved: slug namespacing

Identity of an imported eval is the **eval name within the org**: no
namespacing for now. The upsert-by-name is essential: re-publishing the same
eval lands on the same `Eval` row so runs accumulate into history (trends,
comparison). Collisions (two unrelated suites named the same, or a clash with a
native eval) merge, treated as a name-hygiene problem, acceptable because
`eval.import` is gated on `OrgAgentsManage` (few, trusted publishers per org).
A future optional `project` field can add `(project, name)` identity without
breaking this (absent project = today's behavior); deferred until it bites.
