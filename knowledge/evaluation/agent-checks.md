---
type: Specification
title: "Agent Checks"
description: "Advisory agent config checks (lint, LLM analysis, health checks)."
tags:
  - everruns
  - evaluation
---
# Agent Checks

<!-- Design Decisions:
  - Advisory only: findings never block save, publish, or version creation.
    Enforcement gates were considered and dismissed; suggestions/fixes are the
    escalation path, not errors.
  - Hybrid three-tier pipeline (deterministic rules → LLM checkers → health
    checks) mirrors the converged industry architecture (OpenAI prompt
    optimizer's parallel checker agents, Arbiter arXiv:2603.08993).
  - No request-per-keystroke linting: tier 1 refreshes automatically after
    edits settle (cheap, sync); tiers 2-3 are explicit user actions with visible
    cost/time expectations. No shipping product does continuous background
    analysis; on-demand is the proven interaction model.
  - Tier 2 uses the utility LLM service, not user-configured model providers
    or session secrets (see knowledge/operations/utility-llm.md, "system analysis tasks").
  - Health checks are NOT part of the evals domain UI/entities. They reuse the
    eval runner machinery internally but surface in the agent editor as a
    one-click action. Evals stay user-curated; health checks are
    system-generated and disposable.
  - Findings are computed per resolved config (after harness/capability layer
    merge), cross-layer visibility is our structural advantage over
    prompt-only linters. Cached by agent version config_hash.
  - Extensibility (org-defined rules) is deliberately last: ship built-in
    rules first, learn which ones users mute, then design the custom-rule
    surface (declarative rules + natural-language rubric rules).
  - "Fix with diff" beats raw lint lists: findings carry optional proposed
    replacements rendered as diffs with one-click apply (Anthropic prompt
    improver / OpenAI optimizer pattern).
-->

## Abstract

Agent Checks give users feedback on the quality of an agent configuration
while they build it: structural problems (contradictions, duplication,
verbosity), completeness gaps (tools referenced in the prompt that do not
exist, capabilities with no guidance), and behavioral signals (does the
configured agent actually complete simple tasks). Think "linter plus
mini-evals" for agents and harnesses.

All findings are advisory. The system never blocks saving or publishing; at
most it proposes a fix the user can apply with one click.

## Concepts

### Finding

The atomic unit of feedback, analogous to an eval `Score`.

| Field | Description |
|-------|-------------|
| `rule_id` | Stable identifier, e.g. `prompt.contradiction`, `tools.unknown_reference` |
| `severity` | `warning`, `info`, `suggestion` (no `error`, advisory only) |
| `category` | `structure`, `completeness`, `effectiveness`, `safety`, `cost` |
| `message` | Human-readable explanation of the problem and why it matters |
| `location` | Optional pointer: config field, or prompt span (byte offsets into the authored system prompt) |
| `fix` | Optional proposed replacement text; UI renders as a diff with one-click apply |
| `source` | `builtin` (tier 1), `llm` (tier 2), `health_check` (tier 3) |

### Check tiers

1. **Deterministic rules** (tier 1), pure Rust, run synchronously as part of
   agent preview. Free and instant. Examples: prompt references a tool absent
   from the resolved tool list (and the inverse: enabled capability never
   mentioned), near-duplicate instruction blocks, keyword-class conflicts
   ("be brief" vs "be exhaustive"), agent prompt restating harness/capability
   contributions, over-permissive `network_access`, unused `{{variables}}`,
   prompt length relative to model context.
2. **LLM checkers** (tier 2), narrow single-purpose analysis prompts run
   against the resolved config via the utility LLM service: contradiction and
   interference detection across layers, ambiguity/vagueness, redundancy
   beyond string matching, structure quality, missing per-tool guidance.
   On-demand ("Analyze" action), asynchronous, seconds. Each checker is
   narrowly scoped (contradiction checker, structure checker, ...) rather
   than one mega-prompt, scoped checkers produce higher-precision findings.
3. **Health checks** (tier 3), behavioral mini-evals: the system synthesizes
   a small set of smoke test cases from the agent's own description, prompt,
   and capabilities, executes them as real sessions through the existing eval
   runner machinery, and scores the results. On-demand ("Health check"
   action), minutes, costs real model usage, the UI states this before
   running.

### Check run

An on-demand execution of tier 2 and/or tier 3 against a specific agent
config. Findings from tiers 1 and 2 are not persisted: tier 1 is recomputed
with every preview, and tier 2 is cheap enough (a few bounded utility-LLM
calls) to recompute per Analyze action. Tier-3 results are persisted keyed by
the config hash so repeat views are free and version badges are possible.

Health check runs reuse the durable execution and bounded-concurrency
machinery of eval runs but are not `Eval` entities: they do not appear in
`/evals`, are not user-editable collections, and their generated cases are
disposable. Sessions created by health checks are tagged for filtering, and
case results link to the real sessions for debugging, same debuggability
contract as evals.

## Phases

Decided ordering (Option A → C → B from the design review):

1. **Built-in rules in preview.** Tier-1 rules computed inside
   `POST /v1/agents/preview`; response gains a `findings` array. Editor shows
   a collapsible Checks panel with severity badges and deep links to the
   offending field/span.
2. **LLM analysis + fixes.** `Analyze` action (`POST /v1/agents/analyze`,
   `analyze_agent` command) runs tier-2 checkers on the utility LLM and
   returns merged tier-1 + tier-2 findings. Findings gain span locations and
   proposed `fix` payloads with one-click apply. This phase delivers the
   "too verbose / duplicated / contradictory / poor structure" feedback.
3. **Health checks.** Async behavioral smoke run: the `trigger_agent_health_check`
   command (`POST /v1/agents/{agent_id}/health-checks`) resolves the agent
   config, persists a run row keyed by `config_hash`, and spawns a background
   task that generates cases (utility LLM), runs each as a real session
   through the session/message services, and scores them deterministically
   plus with an LLM judge. `GET .../health-checks/{run_id}` polls status and
   the score card; per-case results link to the real sessions. Runs persist
   in `agent_health_check_runs` (whole run + results as JSONB on one row;
   health checks are not `Eval` entities). Gated on the utility LLM and the
   org default harness being available.

   The editor polls any displayed non-terminal run, including a latest run
   discovered on mount, until it becomes terminal. Analyze and health-check
   provider failures use the shared safe user-facing error taxonomy: they give
   actionable operator guidance without exposing raw provider responses, and
   a failed Analyze attempt does not replace built-in findings.

   The background task is fire-and-forget, so a run is made durable against
   process death two ways: a **boot reaper** transitions every non-terminal
   (`pending`/`running`) row to `failed` on server startup (a fresh process has
   no run in flight, so any such row is an orphan), and a **read-path staleness
   guard** reports a non-terminal run whose `updated_at` is older than a
   generous window (well beyond the runner's max wall-clock budget) as `failed`
   even when the server has not restarted. Together these guarantee a run never
   shows a perpetual `running` spinner.
4. **Extensible rules.** Org-level rule registry: per-rule enable/severity
   config for built-ins, plus two custom rule types, declarative
   (keyword/regex/structural, no code) and natural-language rubrics judged by
   the utility LLM. Admin-managed.

## UX

- **Checks panel** at the top of the default Edit tab, not hidden in Preview or
  rendered as editor squiggles. Preview stays focused on the resolved system
  prompt, tools, and files. Findings grouped by category with severity badges;
  each finding links to its field or highlights its prompt span. Finding counts
  surface as a small badge on the editor tab and on agent cards. Behavioral
  health checks live with the advisory checks in Edit as well.
- **Tier triggers**: tier 1 is implicit (updates with preview); tiers 2 and 3
  are buttons with cost/time expectations ("~30s", "runs N real sessions").
- **Fix flow**: findings with a `fix` show a diff and an Apply button.
  Applying edits the form state; the user still saves explicitly.
- **Dismissal**: users can dismiss a finding for the current config; dismissed
  findings stay hidden until the relevant content changes.

## Non-Goals

- No enforcement: findings never gate save/publish/version-creation.
- No continuous background analysis while typing.
- No public ad hoc LLM endpoint, tier 2 goes through the internal utility
  LLM service only.
- No user-facing dataset management for health checks, curated behavioral
  testing is what Evals are for (`knowledge/evaluation/evals.md`).

## Relationship to existing systems

- **Preview** (`POST /v1/agents/preview`): tier-1 carrier; already computes
  the resolved shape findings are evaluated against.
- **Utility LLM** (`knowledge/operations/utility-llm.md`): engine for tier 2 and for health
  check case generation, under the "system analysis tasks" allowance.
- **Evals** (`knowledge/evaluation/evals.md`): health checks reuse the runner and scorer
  machinery (including `llm_judge` when available) without creating Eval
  entities. If a user wants to keep generated cases, a "promote to eval"
  action is a natural later addition.
- **Agent versions** (`knowledge/runtime-resources/agent-versions.md`): persisted findings and
  health scores key off `config_hash`, enabling per-version badges.
