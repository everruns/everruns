---
type: Specification
title: "Online Evals (Observers)"
description: "Online evals (Observers) over production sessions (proposed)."
tags:
  - everruns
  - evaluation
---
# Online Evals (Observers)

Status: **Phase 1 implemented** (behind the `observers` feature flag) — turn-scope rule **and LLM-judge** scoring of production sessions, with the design options below recording the broader direction. Session/tool scopes, aggregation/alerting, judge usage metering into budgets, and the Phase 2 improvement loop remain proposed.

Phase 1 surface: `Observer` entity (org-scoped, embedded match rules + scorers), `trace_scores` queue, an `ObserverMatchListener` on `turn.completed` that samples and enqueues, and a dual-mode `spawn_observer_worker` that drains the queue (durable `FOR UPDATE SKIP LOCKED` in full mode, in-process in dev mode). Code: `crates/platform/src/observer.rs`, `crates/server/src/domains/observers/`, `crates/server/src/api/observers.rs`, migrations `059_observers.sql` + `070_observer_llm_judge.sql`. API under `/v1/observers`.

Each scorer has a `method`: `rule` (the eval scorer vocabulary) or `llm_judge`. An `llm_judge` scorer carries a `rubric`, an optional `model_id` (defaults to the org's default model), and a `pass_threshold`. The judge call (`crates/server/src/domains/observers/judge.rs`) goes through the **org's own configured model/provider** (resolved via `ProviderResolverService`), returns structured `{value, label, reasoning}`, and stores token/cost accounting on the score. Judge usage is recorded on the `trace_score` row; metering into `usage_journal`/budgets is a follow-up.

A scorer's `model_id` is validated on observer create/update (`ObserverService::validate_model_access`): the model must exist for the org and be enabled, mirroring runtime resolution (`get_model` is org-scoped + enabled-only). Without this an observer could be saved against an inaccessible model and then silently `skip` every score at scoring time. A judge with no `model_id` is always accepted — it resolves the org default at scoring time.

UI: the `observers` feature flag also gates a UI surface (`apps/ui/src/app/(main)/observers/`) — observer list, a create/edit form with a starter scorer catalog and a judge model picker, an "Observe this agent" entry on the agent page, and a per-observer **Quality tab**. The Quality tab aggregates the `/scores` endpoint **client-side** (recent sampled scores → per-scorer pass rate / avg value / daily trend); the durable `fact_trace_score` projection below remains the path to real cross-observer aggregation. Scope is presented as tabs (Answers / Conversations / Tools), but only **Answers (turn)** is wired — `session`/`tool` scopes are shown disabled until the backend implements them.

<!-- Design Decisions (proposed, not yet ratified):
  - Observer is the single user-facing abstraction: create an Observer, configure match rules
    (filters + sampling) and scorers inside it. Scorers/matchers are not standalone entities
    in Phase 1; they are embedded config, extractable later if reuse demands it.
  - Phase 1 is scoring only. No insights, no clustering, no proposals — those form a Phase 2
    subsystem that gets its own name and spec (candidates: Monitor, Insights).
  - Online scoring is a separate subsystem from offline Evals (knowledge/evaluation/evals.md): it observes real
    user traffic instead of creating synthetic sessions. They share the score vocabulary.
  - Observable units: turn output, whole session, AND tool outputs — not just final results.
    Scope is per-scorer, so one Observer can grade answers and tool calls together.
  - Execution is async with two backends behind one trait: durable engine in full mode,
    in-process tasks in dev/in-memory mode — the same task worker runs against different stores.
  - Matching runs on the event stream; scores are stored in their own table and link back to
    the trace (session/turn/tool call) — never appended into the append-only session event log.
  - Scoring never adds latency to user turns.
  - Judge reasoning text is stored, not just scalar scores — it is the raw material for the
    Phase 2 improvement loop (clustering, prompt-learning).
  - Scores feed the reporting layer for aggregation; thresholds feed notifications.
  - Complexity is contained in the UI by progressive disclosure: one-click observer creation
    from an agent with catalog defaults; full editing only for those who open it.
-->

## Abstract

Today evals are offline: a user authors cases, triggers a run, each case creates a synthetic session, scorers grade it (`knowledge/evaluation/evals.md`). That answers "does my agent pass my tests?" but not "how is my agent doing with real users?".

An **Observer** closes that gap. The user creates an Observer, configures inside it *what to watch* (match rules: agent/harness/app/tag filters + sampling) and *how to grade it* (scorers: rule checks and LLM-as-judge rubrics, each scoped to turns, whole sessions, or tool outputs). The Observer watches the live event stream, samples matching traffic, and produces per-trace scores asynchronously.

Phase 1 is deliberately just that — scoring — and is the substrate for what comes after:

1. **Phase 1 — Observers**: per-trace scores on live sessions + aggregation dashboards.
2. **Phase 2 — (unnamed; candidates: Monitor, Insights)**: a subsystem that periodically analyzes scored traces in aggregate (clustering failure modes, missing answers, missing sources, frustration signals) and proposes concrete agent improvements via notifications and a built-in UI. Own spec when Phase 1 lands.

The end state: user creates an agent → their customers use it → the platform scores usage, spots patterns, and proposes improvements → user applies them → repeat.

## Prior art (landscape, June 2026)

All major platforms converged on the same architecture: **rule = target + filter + sampling rate + evaluator(s)**, executed async server-side, scores attached to traces and charted/alerted on.

| Platform | Trigger model | Evaluators | Scope | Closed loop |
|----------|--------------|------------|-------|-------------|
| LangSmith | Automation rules: filter + sampling rate + action; backfill jobs | LLM-judge (prompt templates over run fields), sandboxed no-network Python/JS | run, multi-turn | Insights Agent: hierarchical clustering of ≤1000 traces into usage/failure categories, scheduled reports |
| Langfuse | Evaluator (judge prompt + variables) split from Evaluation Rule (target + filters + sampling + variable mapping, JSONPath) | LLM-judge with structured output; managed catalog (hallucination, helpfulness, toxicity) | observation, trace, session | — (scores + dashboards) |
| Braintrust | Project-level online-scoring rules: scorers + sampling % + SQL filter | autoevals, code scorers, LLM-judge | span or trace | Loop agent: failures → filters → datasets → scorers → suggested prompt edits |
| Arize AX | "Tasks" run every ~2 min on new traces; filters + sampling; backfills | Eval Hub templates + Python code | span, trace, session | Prompt Learning: meta-prompting optimizer driven by textual eval feedback |
| Datadog LLM Obs | Per-evaluation filters + sampling | Custom LLM-judge (span attribute templating), managed evals | span, trace, session | Assessment Criteria normalize scores to pass/fail for monitors |

Notably, the industry trend through 2025–2026 is toward **observation/span-level scoring** (Langfuse now recommends it over trace-level: cheaper, faster, more precise) alongside session-level for multi-turn quality — supporting the requirement that Observers grade tool outputs, not only final answers.

Cross-cutting patterns worth copying:

- **Sampling after filtering** (1–100%), with backfill over historical traces as a companion feature.
- **Async queue with retries and visible execution status**; Langfuse records every judge execution as its own trace for debuggability.
- **Typed score schema** — numeric / categorical / boolean + free-text reasoning — plus a normalization layer (pass/fail) so dashboards and alerts get a uniform signal.
- **Store judge reasoning, not just scores.** Arize Prompt Learning and DSPy GEPA both show textual feedback ("the answer lacked a source citation") optimizes prompts far better than scalars. The reasoning corpus is the input to Phase 2.
- **Cost levers**: sampling, scoring spans instead of whole sessions, cheap judge models, input truncation, judge-cost visibility.

References: LangSmith online evaluations and Insights (docs.langchain.com/langsmith), Langfuse LLM-as-a-judge and scores (langfuse.com/docs/evaluation), Braintrust score-online and Loop (braintrust.dev/docs), Arize online evals and Prompt Learning (arize.com/docs/ax), Datadog LLM Observability evaluations (docs.datadoghq.com/llm_observability).

## What we already have

The building blocks exist; Observers are mostly composition:

| Primitive | Where | Role for Observers |
|-----------|-------|--------------------|
| Persisted immutable event log | `events` table, `crates/core/src/events.rs`, `knowledge/execution/events.md` | Traces are already durable and queryable after the fact (messages, tool calls with results, `llm.generation`, `turn.completed` with usage, `session.idled`). No new capture needed — including tool outputs. |
| `EventListener` | `crates/core/src/event_listeners.rs` | In-process async tap after persistence; how OTel/Braintrust exporters and usage tracking hook in today. Where observer matching runs. |
| Eval scorer rules | `crates/platform/src/eval.rs` | The embedded `Scorer` enum (`contains`, `tool_called`, `turns_within`, …) supplies the rule-method vocabulary for observer scorers too. `llm_judge` is specced (Phase 2 of evals) but not implemented — Observers are the forcing function to build it once, shared by both systems. |
| Durable engine + scheduler | `knowledge/operations/durable-execution-engine.md`, `knowledge/operations/scheduled-tasks.md` | At-least-once background work, multi-instance safe (SKIP LOCKED), cron schedules — production scoring backend. Existing precedent for dev/full duality: the task worker runs with direct in-process stores in dev and gRPC stores in full mode. |
| Tool output distillation | `knowledge/execution/tool-output-distillation.md` | Large tool results are already distilled at capture time — reuse as judge-input truncation for tool-scope scoring. |
| Utility LLM | `knowledge/operations/utility-llm.md` | Candidate judge-model path for platform-internal scoring. |
| Reporting outbox + facts | `knowledge/evaluation/reporting.md` | Async projection pipeline for aggregations (`fact_trace_score` alongside `fact_turn`, `fact_tool_call`). |
| Notifications | `knowledge/operations/notifications.md` | Delivery channel for threshold alerts and Phase 2 improvement proposals. |
| Usage tracking | `knowledge/security/usage-tracking.md` | Judge calls must be metered like any other LLM usage (cost visibility, budgets). |

What does **not** exist yet: message-level user feedback (thumbs up/down), an `llm_judge` scorer, any score storage detached from eval runs, and score aggregation UI.

## Proposed concepts

### Observer

The single user-facing entity. Org-scoped, standard building-block lifecycle (`active → paused → archived`). Everything else — match rules, scorers — is configuration *inside* the Observer (embedded JSONB, like scorers inside EvalCases). If cross-observer scorer reuse becomes a real need, scorers can be extracted into a referenced entity later without breaking the model.

| Field | Description |
|-------|-------------|
| `name`, `description` | Display |
| `match` | Predicates over session/turn metadata: `agent_id(s)`, `harness_id(s)`, `app_id(s)`, session tags, model, errors present, min turns. Empty = all org traffic. Sessions tagged `eval` are excluded by default to avoid scoring synthetic traffic. |
| `sampling_rate` | 0.0–1.0, applied after the match predicates |
| `scorers` | One or more embedded scorer configs (below) |
| `status` | `active` / `paused` / `archived` |

### Scorer config (embedded in Observer)

| Field | Description |
|-------|-------------|
| `key` | Stable name within the observer; becomes the score series name in dashboards |
| `scope` | `turn`, `session`, or `tool` (see below) |
| `method` | `rule` (reuses the eval scorer-rule vocabulary: `contains`, `tool_called`, `turns_within`, …) or `llm_judge` (rubric prompt + variables mapped from the scoped trace slice; judge model reference; structured output `{ value, label?, reasoning }`) |
| `tool_filter` | Tool scope only: which tool names to grade (e.g. just `web_fetch`, just a specific MCP tool) |
| `pass_threshold` | Normalizes `value` to a boolean `pass` — uniform signal for dashboards/alerts (Datadog "assessment criteria" pattern) |

**Scopes — what the scorer sees and when it triggers:**

| Scope | Trigger event | Judge/rule input | Use case |
|-------|--------------|------------------|----------|
| `turn` | `turn.completed` | Input message + final assistant output + tool-call summary | Default. "Was this answer complete? Did it cite sources?" |
| `session` | `session.idled` | Whole conversation transcript | Multi-turn quality: "Did the user get what they came for? Did they repeat themselves?" |
| `tool` | `tool.completed` | Tool name + arguments + (distilled) output, plus the turn's input message for context | "Did retrieval return relevant chunks? Did the search come back empty? Did the tool error?" |

Tool scope is where "observe not only results" lives: failures often originate in a bad tool result two steps before a bad answer. Tool outputs are already persisted on `tool.completed` events, and large ones are already distilled (`knowledge/execution/tool-output-distillation.md`) — the judge reads the distilled form, which doubles as cost control.

One Observer can mix scopes: e.g. a "Support Agent quality" observer with a turn-scope answer-completeness judge, a session-scope goal-completion judge, and a tool-scope retrieval-relevance judge on the KB search tool.

**Naming collision.** `crates/platform/src/eval.rs` already has an embedded `pub enum Scorer` (the per-case scoring rules). The two share the rule vocabulary on purpose, but code needs distinct names — e.g. rename the enum to `ScorerRule`, shared by both eval cases and observers. Decide at implementation time; no backward-compat constraint on internal code.

### TraceScore

The output record. One row per (observer, scorer key, scored unit):

| Field | Description |
|-------|-------------|
| `observer_id`, `scorer_key` | Provenance |
| `session_id`, `turn_id`, `tool_call_id` | What was scored (`turn_id` null for session scope; `tool_call_id` set only for tool scope) |
| `agent_id`, `agent_version_id`, `harness_id` | Denormalized at scoring time — aggregations slice by agent and version ("did the new prompt help?") without joins |
| `value` | 0.0–1.0 (matches eval `Score.value`) |
| `label` | Optional categorical (e.g. `missing_source`, `frustrated_user`, `empty_result`) |
| `pass` | Normalized boolean via the scorer's threshold |
| `reasoning` | Judge explanation text — retained deliberately for Phase 2 |
| `judge_usage` | Token usage of the judge call (also journaled via usage tracking) |
| `status` | `pending` / `completed` / `errored` / `skipped` |

Storage: a new `trace_scores` table that **links back to the trace** (`session_id` / `turn_id` / `tool_call_id`) — users can always click through from a score to the exact conversation or tool call it graded, same debuggability contract as evals. Scores are *not* written into the session event log: events are append-only and replayed by UI/exports, while scores are mutable derived data (re-scoring, observer versioning). Dismissed alternative recorded below.

ID schema: `observer_`, `score_` — final prefixes open.

## Execution

Two requirements shape this: scoring must be **asynchronous on the durability framework** in production, and must **still work when the durable engine is not available** (in-memory dev mode). The codebase already has this exact duality through the unified task worker's direct-store and gRPC-store modes, and observer scoring follows it.

### Trigger (both modes)

An `EventListener` on `turn.completed` / `session.idled` / `tool.completed` evaluates active observers: cheap predicate match + sampling decision. The match data (agent, harness, tags, tool name, error markers) is already on those events or one session lookup away. On match, it hands scoring jobs to the execution backend. Matching itself never runs a judge and never blocks the event path.

### Backend trait, two implementations

```
ScoringBackend::enqueue(job)  // job = (observer_id, scorer_key, session_id, turn_id?, tool_call_id?)
```

- **Durable backend (full mode, recommended production path):** `pending` `TraceScore` rows double as the queue; durable workers claim them via SKIP LOCKED (same pattern as the durable engine), load the trace slice from the `events` table, run the scorer, write results with retry/backoff. At-least-once, multi-instance safe, bounded judge concurrency, per-score execution status visible (the Langfuse/Arize "task log" pattern falls out for free). Backfill = inserting `pending` rows for historical traces matching an observer; the same workers drain them.
- **In-process backend (dev/in-memory mode):** the same jobs run on spawned tokio tasks with a bounded semaphore. No durability — in-flight scoring is lost on restart, no retries, no backfill. Acceptable for dev: the contract is "same behavior, weaker delivery guarantees", matching the task worker's direct-store mode.

A periodic catch-up scan (durable scheduler cron) heals missed enqueues in full mode and powers "apply to past sessions" backfill. Dismissed alternative — scheduler-only scanning as the *primary* trigger (Arize's model) — is recorded below.

### Judge model selection

Two viable paths; this is a real product decision, not just plumbing:

1. **Org's own model drivers** (recommended default): judge calls go through the org's configured providers and are metered/billed via usage tracking like any agent call. Honest cost attribution; works for self-hosted OSS deployments with no platform key.
2. **Utility LLM**: platform-internal key, hidden from the org. Right for built-in "system" observers (e.g. safety screening) but wrong for user-configured judges in an OSS platform — operators without `UTILITY_OPENAI_API_KEY` would lose the feature entirely.

Proposal: scorer config names a model (default: a cheap model from the org's providers); utility LLM reserved for future platform-owned observers.

## UI: containing the complexity

Match rules × three scopes × per-scope scorers is genuinely a lot of surface. The UI strategy is progressive disclosure — the full model exists underneath, but nobody is forced through it:

1. **One-click start.** On the agent page: "Observe this agent" → creates an Observer pre-filtered to the agent, 10% sampling, 2–3 catalog scorers at turn scope (answer completeness, source presence). No form. This is the 80% path.
2. **Catalog, not blank rubrics.** Built-in scorer templates (answer completeness, source/citation presence, user-frustration signal, task completion, hallucination risk, tool-error rate, retrieval relevance) added by toggle. Writing a custom judge rubric is the advanced path, not the entry point.
3. **Scope as a tab, not a concept.** Observer detail page shows scorers grouped under "Answers / Conversations / Tools" tabs rather than asking users to understand scope as configuration. Tool-scope scorers offer a dropdown of tools the agent actually uses (derivable from recent `tool.completed` events) instead of free-text tool names.
4. **Score-first navigation.** The product surface users actually visit is the agent's **Quality tab** (trends, drill-down to traces); observer config is something they touch once. Optimize the reading surface over the authoring surface.
5. **Later (Phase 2 adjacent):** natural-language setup — "watch my support agent for answers without sources" → generated observer config — once the structured model is proven. LangSmith's Insights configuration works this way (guided questions → generated attributes).

## Aggregation and alerting (Phase 1.5)

Per-score rows are necessary but the product value is trends:

- Project `trace_scores` into the reporting layer (`fact_trace_score`) via the existing outbox pattern — avg score / pass rate / label distribution by agent, agent version, harness, observer, scorer key, scope, model, day.
- Agent detail page gets a **Quality tab**: score time series, pass-rate trend, label breakdown, per-tool score breakdown, drill-down from any data point to the underlying sessions/tool calls.
- Threshold alerts: per-observer rule "pass rate < X over window W → notification" via the existing notifications system (new kind, e.g. `observer.threshold_breached`). Webhook delivery can ride Apps/channels later.

## Phase 2 sketch: the improvement loop (own name, own spec)

Phase 2 is a distinct subsystem — naming candidates: **Monitor**, **Insights** — specced separately once Phase 1 lands. Phase 1 deliberately produces the substrate it consumes: sampled traces with scores, labels, and **judge reasoning text**, at answer, conversation, and tool granularity.

- **Insight job**: scheduled (cron) durable workflow per agent that takes the last N scored traces (failing ones first), clusters them by label + reasoning embedding into failure modes — "12% of sessions: answer lacked a source", "8%: user repeated the question", "30% of KB searches returned nothing" (LangSmith Insights pattern).
- **Improvement proposals**: a second pass turns clusters into concrete suggestions — system-prompt diffs, missing knowledge-base content, missing tools/capabilities, eval cases to add (Braintrust Loop pattern). Textual judge reasoning is the optimizer input (Arize Prompt Learning / GEPA result: English feedback beats scalar scores). Tool-scope scores let proposals point below the prompt: "retrieval quality, not prompt wording, is the bottleneck".
- **Surface**: notification ("Weekly insight for Support Agent: 3 improvement proposals") linking to a built-in UI; each proposal shows evidence (linked sessions), the suggested change, and one-click actions: apply prompt diff (creates a new agent version, `knowledge/runtime-resources/agent-versions.md`), add suggested eval case (closing the loop into offline evals as the regression gate).
- The full loop: production traffic → trace scores → insight clusters → proposal → applied change → new agent version → offline eval gate + continued online scoring of the new version.

Phase 2 changes no Phase 1 storage decisions except the requirement (already encoded) to keep reasoning text and version stamps.

## Privacy and cost guardrails

- Judges read raw production content. Observers must be **explicitly created** (nothing scored by default), and config is org-scoped like everything else (`knowledge/security/multitenancy.md`). Exporter-style redaction modes (cf. Braintrust listener's content controls) should apply to what judge prompts may include.
- Sampling default conservative (e.g. 10%); per-org cap on judge calls/day; judge usage metered through `usage_journal` so budgets (`knowledge/security/budgeting.md`) can bound it. Tool scope multiplies score volume (many tool calls per turn) — per-scope sampling or tool filters are the lever.
- Judge inputs are truncated; tool outputs use the already-distilled form (`knowledge/execution/tool-output-distillation.md`).
- Sessions created by offline eval runs (tagged `eval`) are excluded by default; observer-triggered judge calls must never themselves be matched (no recursion).

## Dismissed options

- **Scores as session events**: would reuse the event pipeline, but scores are mutable derived data (re-scoring, observer versioning) and the session event log is append-only and replayed by UI/exports — every consumer would need to filter score events. Scores link to the trace instead (`session_id`/`turn_id`/`tool_call_id` on `trace_scores`).
- **Reuse `EvalRun`/`EvalCaseResult` for online scores**: tempting (shared UI), but the cardinality and lifecycle differ — runs are user-triggered finite batches; observers are unbounded streams. Shared vocabulary (`Score` shape, scorer-rule enum) yes; shared tables no.
- **Standalone Scorer + Matcher entities** (earlier draft): Langfuse-style factoring with reusable scorers bound by separate matcher rules. More flexible, but two entities and an indirection for day one; Observer-with-embedded-config keeps one mental model ("create an observer, set it up") and the extraction remains possible later. The built-in catalog covers most reuse in practice.
- **Scheduler-only scanning as primary trigger** (Arize's model): simplest durable design, but minutes of latency, watermark bookkeeping, and scan cost on idle traffic; kept only as catch-up/backfill.
- **External-platform-only** (point users at Braintrust/Langfuse online scoring via the existing exporters): zero build, but the Phase 2 loop (proposals that mutate agent versions, eval-case generation) requires the scores and reasoning to live in-platform. Exporters remain complementary.

## Open questions

1. Code naming vs the existing embedded `Scorer` enum in `eval.rs` (proposal: rename enum to `ScorerRule`, shared by eval cases and observers).
2. Match predicate expressiveness for Phase 1: fixed structured fields (agent/harness/app/tags/model/errors) vs a filter DSL. (Proposal: structured fields; a DSL can come later — every platform started structured.)
3. Phase 1 scope set: all three (turn/session/tool) from the start, or turn first with session/tool fast-following? Tool scope is the differentiator but also the volume risk.
4. Built-in scorer catalog contents and whether catalog judges count against org budgets.
5. Message-level user feedback (thumbs up/down) — not strictly required, but every platform treats explicit feedback as the highest-signal filter ("score traces where users were unhappy"). Likely a small, high-leverage prerequisite worth its own line item.
6. Observer versioning: re-score on edit, or only score forward? (Proposal: forward-only + explicit backfill.)
