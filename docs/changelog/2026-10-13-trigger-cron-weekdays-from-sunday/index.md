---
title: Schedule triggers read cron weekdays from Sunday as 0
description: A 5-field cron such as 0 9 * * 1-5 on an Agent trigger or durable schedule now runs Monday to Friday, and 0 18 * * 0 runs on Sundays.
date: 2026-10-13
kind: fix
significance: minor
products: [platform, cloud, framework]
surfaces: [ui, api, cli]
links:
  docs: [/features/agent-triggers/]
  prs: [everruns/everruns#4703, everruns/everruns#PRNUM]
---

Agent schedule triggers and durable schedules read the weekday field of a
5-field cron one day off: `0 9 * * 1-5` ran Sunday to Thursday, and
`0 18 * * 0` was rejected. Weekdays now count 0-6 from Sunday, with 7 also
Sunday, the same as session schedules and standard cron.

The fix applies when a trigger or schedule is created or its cron is changed.
Existing ones keep the days they have been running on; to switch one to the
standard reading, enter its 5-field cron again. The cron editor now describes
and previews a stored 7-field expression with the days it actually runs on, so
an older trigger saved as `0 0 9 * * 1-5 *` reads "Sunday through Thursday"
rather than "Monday through Friday". See
[Agent triggers](/features/agent-triggers/).

`#[schedule]` crons in an `everruns-serve` app read 5-field weekdays the same
standard way, so they run on the same days locally and when deployed. See
[Serve](/framework/serve/).
