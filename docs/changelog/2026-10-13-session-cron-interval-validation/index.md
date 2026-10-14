---
title: Recurring schedules reject unverified cron intervals
description: Session schedules now reject invalid cron and insufficient future occurrences at admission, with the same parser used to calculate triggers.
date: 2026-10-13
kind: fix
significance: minor
products: [platform, cloud, framework]
surfaces: [api, code]
links:
  docs: [/capabilities/session-schedules/]
---

Recurring session schedules now reject cron expressions whose minimum interval
cannot be verified. Invalid cron and expressions with fewer than two future
occurrences return an actionable rejection before schedule storage, instead of
passing the interval check without evidence.

Admission and trigger calculation use the same cron parser in the hosted
Platform and embedded Framework. Standard weekday numbering, supported cron
forms, timezone interpretation and schedule-count limits are preserved. For a
single future run, use a one-shot schedule with `scheduled_at` instead of a
finite recurring cron.

See [Session schedules](/capabilities/session-schedules/).
