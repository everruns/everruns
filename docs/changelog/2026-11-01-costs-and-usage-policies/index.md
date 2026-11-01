---
title: See what agents cost and limit usage with usage policies
description: Settings → Costs shows every usage with its cost and where that cost came from, holds your own price list, and replaces budgets with usage policies that notify, ask to continue, or stop.
date: 2026-11-01
kind: feature
significance: major
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/features/costs/]
---

**Settings → Costs** is new. It records every model call and every stretch of
sandbox running time with a cost, and every amount says where its number came
from: your rate, the provider, the model catalog, or "not priced". Unknown
costs are shown as unknown, never as $0 and never as a token count read as
dollars, which a dollar budget used to do for models without a catalog price.

**Price list:** set your own internal rates per provider, model or sandbox
size, as a fixed price or a multiple of the provider's price. Sandboxes are
priced only by your rates.

**Usage policies** replace budgets. A policy covers the organization, a person,
an agent, a channel or unattended work, and runs steps as usage grows: notify
owners and admins, ask the person using the agent before continuing (a
**Continue this turn** button in chat, for that session), or stop. Days, weeks
and months follow a new organization timezone setting.

**In the API:** `/v1/budgets` accepts a `policy` object with `steps`;
`/v1/cost-rates` holds the price list; `GET /v1/costs/summary` returns cost by
where it came from, source, agent, person and day. The `credits` budget
currency is gone: existing credit budgets were converted to tokens (× 1,000).
See [Costs and Usage Policies](/features/costs/).
