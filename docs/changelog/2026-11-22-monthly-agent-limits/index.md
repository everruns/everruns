---
title: Agent limits reset every month
description: Organizations can create 500 agents each month instead of keeping 500 at once, and owners and admins can raise the allowance to 100,000.
date: 2026-11-22
kind: improvement
significance: minor
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/features/ui/]
---

Archived agents no longer use up an organization's agent allowance for good. Each organization can now create 500 agents per UTC calendar month, and the count starts again at midnight UTC on the first of the month. Owners and admins can set a limit from 1 to 100,000 in **Settings → Limits**, next to the session limit. Built-in agents never count.

This fixes organizations that create short-lived agents, such as evaluation or deployment checks: their archived agents used to fill the cap, so new agents failed with "Agent limit reached".

To change the limit with the API, send this body to `PATCH /v1/orgs/{org}`.

```json
{
  "monthly_agent_limit": 2000
}
```

See [organization settings](/features/ui/#monthly-agent-limit) for details.
