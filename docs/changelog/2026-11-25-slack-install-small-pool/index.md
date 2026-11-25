---
title: Add to Slack no longer fails with a server error on small pools
description: One-click Slack install, its OAuth return and channel removal hold one database connection for their locks, so a two-connection pool is enough.
date: 2026-11-25
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/capabilities/slack/]
---

Pressing **Add to Slack** could wait five seconds and fail with HTTP 500 on
every attempt. The install locks the agent and the channel so that two clicks
never create two Slack apps, and each lock held its own database connection.
On a server whose request pool has two connections, the install then had none
left to read the channel, and it gave up.

Both locks now share one connection, in the install, its OAuth return, the
reconnect fix and channel removal, so the same server finishes them. See
[Slack](/capabilities/slack/).
