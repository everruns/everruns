---
title: Session schedules no longer fire twice on multi-replica deployments
description: Everruns servers running more than one replica now claim each due session schedule atomically, so a scheduled message is sent once.
date: 2026-09-11
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [operations]
release: "0.26.0"
links:
  docs: [/capabilities/session-schedules/]
  prs: [everruns/everruns#3527]
---

On a deployment with more than one server replica, two replicas could both pick
up the same due session schedule and send its message twice. Schedules are now
claimed atomically, so exactly one replica fires each one. Single-replica
deployments were not affected, and no configuration change is needed.

See [Session schedules](/capabilities/session-schedules/). [#3527](https://github.com/everruns/everruns/pull/3527)
