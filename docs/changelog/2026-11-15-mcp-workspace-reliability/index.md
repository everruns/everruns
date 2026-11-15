---
title: MCP workspaces preserve drafts and recover connections
description: Session data calls no longer reopen the workspace, repeated entry results preserve drafts and tabs, and timed-out host connections can be retried.
date: 2026-11-15
kind: fix
significance: notable
products: [platform, cloud]
surfaces: [ui, agent]
links:
  docs: [/features/mcp/]
---

MCP session data calls now return results without opening another workspace.
Ask your connected client to open a session when you want its interface:

```text
Run the Research Assistant, check its progress, then open its session.
```

Repeated workspace results refresh data and preserve drafts and the selected tab, including
through theme and fullscreen changes. Switching organizations clears the
previous workspace state. If the host connection times out while opening,
choose **Retry connection**. Pending requests are cleaned up when the view
closes; writes are never retried automatically. See [MCP Apps](/features/mcp/#in-chatgpt-and-codex).
