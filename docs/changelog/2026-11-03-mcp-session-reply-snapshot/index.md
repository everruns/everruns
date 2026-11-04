---
title: MCP session status keeps delivered replies visible
description: MCP status polling returns the latest delivered reply even after long tool runs, filtered event requests, or polling past the reply event.
date: 2026-11-03
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [agent]
links:
  docs: [/features/mcp/]
---

MCP clients polling `session_get_status` now receive the latest visible agent
reply in `latest_output`, including answers delivered through `send_message`.
Replies no longer disappear when they fall outside the returned event page.
Event summaries include their types, and incremental polls advance through
events without skipping or repeating pages.

The field stays available when polling with `since_event_id` or filtering to
status events. Private commentary and unsuccessful sends are excluded. It is a
session snapshot: a turn that says nothing leaves the previous reply available,
and a session with no visible text reply returns `null`.

See [MCP polling for replies](/features/mcp/#polling-for-replies) for the event
cursor and reply semantics.
