---
title: Concurrent Yolop sessions share MCP OAuth refreshes safely
description: Cross-process coordination prevents competing token rotations and keeps logout from being undone by an in-flight refresh.
date: 2026-11-09
kind: fix
significance: minor
products: [yolop]
surfaces: [cli, agent]
links:
  prs: [everruns/yolop#929]
---

Yolop sessions using the same MCP OAuth connection now coordinate token refreshes across processes. A waiting session reads the latest saved credentials before refreshing, so two sessions cannot spend the same rotating refresh token concurrently.

Logging out or signing in again while a refresh is running takes precedence: the older response cannot restore or replace those credentials. Lock waits respect cancellation and have a deadline. Unreadable credential files fail closed instead of reusing stale tokens. No configuration change is required.
