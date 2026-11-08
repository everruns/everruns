---
title: Schedules fire in their own timezone
description: A schedule set to a timezone such as America/Chicago now runs at that local time, across daylight saving changes, instead of that time in UTC.
date: 2026-11-08
kind: fix
significance: minor
products: [platform]
surfaces: [ui, api, cli]
---

Every schedule trigger and durable schedule has a timezone, but the next run was
computed in UTC. An agent set to run at `0 10 * * *` in `America/Chicago` ran at
10:00 UTC, six hours early, and the usual workaround was to set the timezone to
UTC and shift the hour by hand.

Schedules now read their cron in the timezone you set and keep that local time
when daylight saving time starts or ends. Changing a schedule's cron or timezone
moves its next run right away, and an unknown timezone name is rejected when you
save the trigger.

A schedule that already has a next run queued fires once more at that time, then
follows its timezone. Saving the trigger moves it immediately.
