---
title: Organizations can set a monthly session limit
description: Organizations can create 10,000 sessions each month by default, and owners and admins can raise the allowance to a maximum of 100,000.
date: 2026-11-21
kind: improvement
significance: minor
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/features/ui/]
---

Each organization can create 10,000 sessions per UTC calendar month by default. Owners and admins can set a limit from 1 to 100,000 in **Settings → Limits**. The allowance resets at midnight UTC on the first day of each month. Archived and deleted sessions still count for the month they were created.

To change the limit with the API, send this body to `PATCH /v1/orgs/{org}`.

```json
{
  "monthly_session_limit": 25000
}
```

See [organization settings](/features/ui/#monthly-session-limit) for details.
