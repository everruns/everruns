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
told. This concept owns the first two, detection and the limits table; lean
runs follow in a later change (see Rollout).

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
(`mark_triggered`), `crates/server/src/background/session_scheduler.rs`.

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

Source: `crates/server/src/domains/health_issues/background_work.rs`,
`crates/server/src/storage/background_work.rs`.

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
3. Lean runs: `web_fetch` probe monitors with a wake condition, a small model
   view for scheduled turns. The Muse cache fix shipped separately.

Related: [Budgeting](../security/budgeting.md),
[Actionable Health Issues](health-issues.md),
[Session tasks](../runtime-resources/session-tasks.md).
