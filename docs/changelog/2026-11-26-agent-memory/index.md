---
title: "Agents remember what you decided"
description: "Agent memory is rebuilt: decisions, rules, facts, preferences and blocked items kept as Markdown files in stores for the agent, its team, its parent, you and the organization."
date: "2026-11-26"
kind: feature
significance: major
products: [platform, cloud]
surfaces: [ui, api, cli, agent]
links:
  docs: [/features/memory-scopes/, /capabilities/memory/]
---

Tell a coordinator "allow it" on a dependency PR, and next week's run follows that instead of
asking again. Agents `remember` decisions, rules, facts, preferences and blocked items with the
exact words they rest on, as plain Markdown files:

```text
/memory/team/decisions/everruns-5528.md
/memory/user/preferences/reply-length.md
```

Stores say who shares a memory: the agent, its team (a coordinator and every thread it starts),
its parent agent, you alone, the organization, or a [named memory](/capabilities/memory/). Each is
a switch on the Memory capability. Every change is kept, and **Memory** under Blocks shows entries
and history; see [how memory works](/features/memory-scopes/).
