---
title: See what agents cost and limit usage with usage policies
description: Settings → Costs shows each usage's cost and where it came from, keeps your own price list, and turns budgets into usage policies that notify, ask or stop.
date: 2026-11-01
kind: feature
significance: major
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/features/costs/]
---

**Settings → Costs** records every model call and sandbox run with a cost, and
says where each number came from: your rate, the provider, the model catalog,
or "not priced". An unknown cost stays unknown instead of a token count read as
dollars.

Your **price list** sets internal rates per provider, model or sandbox size.
**Usage policies** replace budgets: as usage grows they notify owners and
admins, ask the person in chat to continue, or stop. Days, weeks and months
follow a new organization timezone. Credit budgets were converted to tokens.

```bash
curl -X POST "$EVERRUNS_URL/v1/budgets" -H "Authorization: Bearer $EVERRUNS_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"subject_type":"agent","subject_id":"agt_…","currency":"usd","limit":20,
       "period":{"type":"calendar","unit":"day"},
       "policy":{"steps":[{"at_percent":75,"action":"ask"},{"at_percent":100,"action":"stop"}]}}'
```

See [Costs and Usage Policies](/features/costs/).
