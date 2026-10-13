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
  prs: [everruns/everruns#4703, everruns/everruns#4708]
---

Agent schedule triggers and durable schedules read the weekday field of a
5-field cron one day off: `0 9 * * 1-5` ran Sunday to Thursday, and
`0 18 * * 0` was rejected. Weekdays now count 0-6 from Sunday, with 7 also
Sunday, the same as session schedules and standard cron.

Existing triggers keep the days they run on; to switch one, enter its 5-field
cron again. The cron editor now labels a stored 7-field expression with the
days it actually runs, so an older `0 0 9 * * 1-5 *` reads "Sunday through
Thursday". See [Agent triggers](/features/agent-triggers/).

`#[schedule]` crons in an `everruns-serve` app read 5-field weekdays the same
way, locally and when deployed. See [Serve](/framework/serve/).
