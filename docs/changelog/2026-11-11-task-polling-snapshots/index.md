---
title: Shorter responses for unchanged task polls
description: Reuse a task list snapshot to avoid receiving unchanged background task lists while preserving progress and input requests.
date: 2026-11-11
kind: improvement
significance: minor
products: [framework, platform, cloud]
surfaces: [agent]
links:
  docs: [/capabilities/sub-agents/]
---

The `list_tasks` tool returns a snapshot that agents can pass as `since` on the
next poll with the same filters. When nothing changed, it returns a short
`unchanged` response instead of repeating every background task. Progress,
state details, pending input requests, and outcomes still produce a fresh list.
Heartbeat timestamps alone do not.

Omit `since` to retrieve the list again, or use `get_task` for full details.
Snapshots work across turns without relying on a particular worker process.
