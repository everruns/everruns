---
title: Scheduled triggers recover without duplicate dispatch
description: Durable activity schedules enqueue each occurrence atomically, and agent-trigger retries reuse the committed session and input.
date: 2026-11-17
kind: fix
significance: minor
products: [platform]
surfaces: [api]
links:
  docs: [/features/agent-triggers/]
---

Agent schedule triggers now reuse the result of a committed dispatch when a worker
retries after losing its acknowledgement. The session, input and retry receipt
commit together, so an interrupted dispatch can retry without leaving a second
session or appending the same scheduled input again.

Durable activity schedules also commit their queued task, execution record and
next-trigger time together. This closes the restart window that could enqueue the
same occurrence twice after the first task had been claimed.

This covers Platform agent-trigger dispatch, not exactly-once execution of an
agent's external tools. Manual fires remain distinct invocations. Generic
workflow-target schedules are unchanged. The fix requires the updated server,
worker and database migration; it does not indicate a Cloud deployment.
