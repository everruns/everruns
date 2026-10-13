---
title: Agents now see why a session schedule cron was rejected
description: An invalid cron or timezone in create_schedule now returns the reason to the agent instead of an internal error, and Sunday crons obey the interval.
date: 2026-10-12
kind: fix
significance: minor
products: [platform, cloud, framework]
surfaces: [api, code]
release: "0.48.0"
links:
  docs: [/capabilities/session-schedules/]
  prs: [everruns/everruns#4615]
---

When an agent called `create_schedule` with an invalid cron expression or an
unknown timezone, the tool failed with "Internal server error", so the model
could not tell what to fix and the server logged it as an error. The tool now
returns the reason, such as `Invalid IANA timezone 'Mars/Base'`, and the agent
can retry with a corrected schedule.

Standard 5-field crons that name weekdays by number, such as `0 18 * * 0` for
Sunday evenings, are read the same way everywhere, including in embedded
Framework schedules. The 5-minute minimum interval now also applies to them.

See [Session schedules](/capabilities/session-schedules/). [#4615](https://github.com/everruns/everruns/pull/4615)
