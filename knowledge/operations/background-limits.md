---
type: Specification
title: Background Work Limits
description: Lifetime, stop condition and spend caps on unattended work, so a schedule no person watches cannot run or spend without end.
tags:
  - everruns
  - operations
  - schedules
  - budgets
---

# Background Work Limits

## Why

On 2026-10-18 a test agent created a `*/5` session schedule to poll for an
approval that never came. It fired 3,766 times over 13 days and spent about
$106 before a person noticed. Five safeguards were missing: schedules had no
end, unattended work had no default spend cap, every fire ran on the whole
session history, nothing noticed the runs achieved nothing, and nobody was
told. This concept owns the first two, detection, polling without the model,
the short model view for scheduled runs and the limits table.

## Lifetime

- A schedule an agent creates as recurring must state how long it runs
  (`ends_at`, `max_runs`, or both) and when it is done (`stop_when`, one
  sentence). There is no silent default: a call without them is rejected, and
  the rejection names the limit. Whichever end comes first stops it.
- Agent ceiling: 7 days and 96 runs. At the 5 minute minimum interval 96 runs
  is 8 hours, so a longer wait needs a longer interval.
- One-shot schedules fire once (`max_runs` 1).
- The limits are visible to the model before it calls, in the tool
  description and as `maximum` in the argument schema, again in validation
  errors, and the tool result echoes what was granted.
- Every fire of a recurring schedule appends the run number, end, stop
  condition, spend so far and how to cancel to the injected message. The last
  run says it is the last.
- A schedule that reaches its end is disabled with `stop_reason = ended`. A
  person can extend `ends_at`, `max_runs` and the budget, or resume it, from the
  session's schedules; a person is not held to the agent ceiling. Resuming
  clears `stop_reason`.
- Agent triggers are configured by people and have no lifetime; their spend
  counts as unattended.

Source: `crates/contracts/src/runtime/session_schedule.rs`
(`resolve_agent_schedule_bounds`), `crates/server/src/domains/session_schedules/service.rs`
(`mark_triggered`), `crates/server/src/background/session_scheduler/mod.rs`.

## Spend

Unattended means no person started the turn: a schedule fired it (including
automatic retries, which are one-shot schedules) or an agent trigger started
the session.

- **`session_schedule` budget**, keyed on the schedule's `sched_` id, created
  with every agent schedule: $2 by default, up to $10 when the agent passes
  `budget_usd`. A schedule fire's input message records its schedule on
  `runtime_invocations.schedule_id`; metering charges that turn's generations
  to the schedule, and the loop gate reads the session's latest invocation to
  stop a turn mid-way. Human turns in the same session never touch it.
- **`org_unattended` budget**, keyed on the org's public id, a calendar day
  period: $20 per day by default. Created lazily on the first unattended turn;
  one live row per org (partial unique index).
- Before firing, the scheduler checks both. A spent schedule budget disables
  the schedule with `stop_reason = budget_exhausted`. A spent org budget skips
  the fire without counting it: a recurring schedule moves to its next time, a
  one-shot waits for midnight UTC.
- Both fail open when the budget store is unreachable, like the loop gate.

Source: `crates/server/src/domains/budgets/service.rs` (`attach_unattended`,
`unattended_fire_gate`), `crates/server/src/storage/background_limits.rs`.

## Detection

Health issues in **Settings → Health** (see [Actionable Health Issues](health-issues.md))
for unattended work that goes wrong. Each is about one subject, kept in
`health_issues.subject_id`, with the numbers in `evidence`.

| Code | Subject | Opens when | Resolves when |
|---|---|---|---|
| `schedule.runaway` | schedule | its runs ended the same way 24 times in a row | a run ends differently, or a person resumes or cancels it |
| `schedule.budget_exhausted` | schedule | the scheduler stops it at its budget | a person resumes or cancels it |
| `org.unattended_budget` | org | today's unattended budget is spent | the next day, or the cap is raised |
| `agent.unattended_spend_spike` | agent | its unattended spend over 24 hours passed $5 and 3× its average day over the six days before | the condition no longer holds |
| `session.context_bloat` | session | its last 20 model calls averaged more than 200k prompt tokens with under 10% cached | the condition no longer holds, or it has no calls for an hour |
| `session.tool_error_loop` | session | among its last 40 tool calls of the last hour, the same call (tool and target) failed 8 times | the condition no longer holds |
| `session.repeated_tool_calls` | session | among those calls, the same call was made 20 times | the condition no longer holds |
| `session.spend_spike` | session | it spent more than $2 in the last hour and 10× its agent's median session-hour over the week before (judged once the agent has 12 such hours) | the condition no longer holds |
| `session.suspected_abuse` | session | a decision model answered yes, with probability 0.85 or more, that the session is used for spam, scraping at scale, credential attacks, crypto mining, phishing or malware | a later read answers no, or the session makes no model call for a day |
| `session.agent_stuck` | session | the same, at 0.8, that the agent repeats itself without progress | as above |
| `session.followed_injection` | session | the same, at 0.8, that the agent acted on instructions from a tool result or fetched content rather than its user | as above |

- **Runaway schedules.** Before each fire the scheduler fingerprints the
  previous fire's final answer (lowercased, digits and spacing collapsed, so a
  changing timestamp still matches) and counts identical answers in a row on
  the schedule (`same_outcome_runs`). A fire whose turn has not finished is not
  counted. From 24 on, each fire's message warns the agent how many identical
  runs are left and around when it stops, and suggests cancelling or a longer
  interval. At 48 the schedule stops with `stop_reason = runaway` and no turn
  starts. A person resuming the schedule resets the count.
- **Who is told.** An issue opening sends a notification (kind `health.issue`)
  to the schedule's owner and the agent's authors, the members who changed the
  agent in its history; the org's owners for the org budget, and whenever no
  owner or author is known. A warned schedule that is then stopped shows its
  notification again. Notifications carry no text from the agent's answers.
- A 15 minute sweep (`background_work_health_sweep`) resolves schedule issues,
  and opens and resolves the org budget, spend spike and context bloat issues.
  **Check again** runs the issue's part of it. Only the runaway stop pauses
  anything; the rest report.

- The three `session.*` behavior detectors apply to every session, attended
  or not, except eval sessions. The spend baseline is the agent's own median,
  not a fixed bar, so an agent that is always expensive is not flagged and a
  cheap one that suddenly is, is. They report to the agent's authors and stop
  nothing. The notification names the tool and amounts, never the call's
  target.
- **Session classifiers** (layer 2) read every session a behavior detector or
  classifier flagged, every 6 hours while it stays flagged, plus a random 10
  active sessions per sweep, each read at most once a day. A claim row per
  session (`session_classifications`) keeps replicas from reading a session
  twice. The model is the one the org's `system_decisions` setting selects.
  Sampling covers only orgs on the deployment's decision service, and only
  when the deployment has one: an org that answers decisions on its own
  provider account pays per call, so it is read only when flagged. The
  classifier reads the last six messages (1,500 characters each) and the
  tools and targets of the last 40 tool calls; nothing of that is stored. The
  issue keeps the probability and the model, and the notification says which
  question was answered yes. Findings notify only: nothing is stopped or
  suspended on a classifier's word, and a person confirms anything further.
- **System doctor** reports `observability.detection`: a warning when the
  sweep failed or has not finished for 40 minutes, so a detector that stopped
  is visible. `observability.classifiers` warns while any session is
  suspected of abuse, so a deployment's operator hears of it, and when the
  last classifier sweep could not read any session.

Source: `crates/server/src/domains/health_issues/background_work.rs`,
`crates/server/src/domains/health_issues/behavior.rs`,
`crates/server/src/domains/health_issues/classifiers.rs`,
`crates/server/src/storage/background_work.rs`,
`crates/server/src/storage/behavior_detectors.rs`,
`crates/server/src/storage/session_classifications.rs`.

## Polling without the model

Most runaway schedules are polls: call an endpoint, act only when the answer
changes. A poll should cost fetches, not model turns.

- `spawn_background` with a `schedule` and a `schedule.wake_when` creates a
  **probe monitor** on a tool the scheduler can run itself (`web_fetch`,
  `get_current_time`; `MONITOR_PROBE_TOOLS`). Each fire runs the tool directly,
  with no agent turn and no model tokens, and records one short line on the
  monitor's thread.
- `wake_when` is a JSON pointer into the tool result (a JSON response body is
  parsed under `/json`) plus exactly one of `equals`, `not_equals` or `changed`.
  The agent gets one turn when the condition becomes true, with the result in
  the fire message; a condition that stays true does not wake it again, and
  `changed` wakes once per new value. A failed call is `{"error": …}`, so a
  broken poll wakes the agent once instead of failing silently.
- The probe runs with the session's tool context: the folded network access
  list and the host egress boundary scoped to the org and session, the same
  controls an agent turn applies (THREAT TM-AGENT-018). If that context cannot
  be built, the probe does not run. Bound credentials, files and sandboxes are
  not available to probes.
- Lifetime and budget limits apply to the monitor's schedule as to any other;
  quiet fires cost nothing, so only woken turns spend.

For the Shipmail case: 3,766 fetches, no model tokens, and one turn when the
approval arrives. Source: `crates/contracts/src/runtime/monitor_probe.rs`,
`crates/server/src/background/monitor_probe.rs`.

## Short model view for scheduled runs

A fire that does need the model should not pay for the whole session. The
Shipmail poll sent about 380k prompt tokens per fire because each fire ran on
2,349 earlier messages.

- When a turn's input is a fire of a recurring session schedule (the reserved
  message origin, kind `schedule` with a run counter), the compaction
  capability's model view shows the model only: the leading system messages
  (the latest checkpoint summary, when one exists), the last 10 messages of the
  same schedule's earlier fires, kept as whole fires, newest first, and the
  current fire with this turn's work. The system prompt is added as usual.
- Everything else, the chat around the schedule and other schedules' fires, is
  left out of the prompt only. Storage stays lossless, and the fire message
  says that older messages are stored but not shown.
- One-shot fires and every other turn keep the full history: a reminder fires
  once and usually depends on the conversation that set it. Clients cannot
  label their own messages as fires, because the origin key is stripped from
  caller metadata.
- Because the view hides the conversation, the schedule tools tell the agent
  to write a self-contained description.

The window is fixed at 10 messages. Source:
`crates/core/src/builtins/compaction/scheduled_view.rs`.

## Limits

| Limit | Default | Org may set |
|---|---|---|
| Minimum interval between fires | 300 s | no |
| Active schedules per session / org | 5 / 100 | no |
| Agent schedule lifetime | 7 days | 1 to 7 days |
| Agent schedule runs | 96 | 1 to 96 |
| Default schedule budget | $2 | any positive, not above the largest |
| Largest schedule budget an agent may set | $10 | any positive |
| Unattended spend per org per day | $20 | any positive, or no cap |
| Identical runs before warning / stop | 24 / 48 | no |
| Spend spike floor / factor | $5 / 3× | no |
| Context bloat: calls / tokens / cached | 20 / 200k / 10% | no |

Defaults are constants in code, not environment variables, so every
deployment starts safe. Org admins change them in **Settings → Limits**
(`organization_settings.background_limits`, PATCH `/v1/orgs/{org}`). Changing
the daily cap updates the live `org_unattended` budget; removing it disables
that budget. A later SaaS hook can supply per-plan defaults.

The local (embedded) schedule store asks the agent for the same bounds and
records them, but does not meter spend.

## Rollout

1. Lifetime, `stop_when`, schedule and org unattended budgets, Settings →
   Limits (this concept).
2. Detection (this concept).
3. Lean runs: `web_fetch` probe monitors with a wake condition, and the short
   model view for scheduled runs (this concept). The Muse cache fix shipped
   separately.

Related: [Budgeting](../security/budgeting.md),
[Actionable Health Issues](health-issues.md),
[Session tasks](../runtime-resources/session-tasks.md).
