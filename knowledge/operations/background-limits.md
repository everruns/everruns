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
told. This concept owns the first two and the limits table; detection and
lean runs follow in later changes (see Rollout).

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
2. Detection: a schedule whose runs end the same way is warned in context
   after 24 runs and paused after 48; health issues for runaway schedules,
   budget stops, spend spikes and context bloat, notifying the schedule's
   owner and the agent's authors (org owners for the org budget).
3. Lean runs: `web_fetch` probe monitors with a wake condition, a small model
   view for scheduled turns, and the cache telemetry fix for Muse via
   OpenRouter.

Related: [Budgeting](../security/budgeting.md),
[Actionable Health Issues](health-issues.md),
[Session tasks](../runtime-resources/session-tasks.md).
