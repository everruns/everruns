---
title: Costs and Usage Policies
description: See what agents cost your organization, set your own internal rates, and limit usage with policies that notify, ask before continuing, or stop.
sidebar:
  label: Costs and Usage Policies
appliesTo: [platform, cloud]
---

**Settings → Costs** answers two questions: what did our agents cost, and what
stops them from costing more than we want.

## Costs

Everruns records every model call and every stretch of sandbox running time,
whether or not anything limits it, and gives each one a cost. Every amount says
where its number came from:

| Label | Where the number came from |
|---|---|
| Your rate | A rate from your price list |
| Provider reported | The provider sent its cost with the response (OpenRouter does) |
| List price estimate | The model catalog price for the model, cache-aware |
| Billed by host | A host that bills usage itself reported what it charged |
| Not priced | No price anywhere. The cost is unknown, never shown as $0 |

When some usage is not priced, totals say "partial" and the Overview tab says
how many records are missing a price.

The **Overview** tab shows this month (in your organization's timezone) by
agent, by source (built-in models, each provider on your own keys, sandboxes)
and by who started the work, with unattended work (schedules and triggers) on
its own line. An agent's integrations panel shows its own month so far.

## Price list

The **Price list** tab holds your own internal rates. A rate covers model
tokens or sandbox running time, optionally only one provider, model, sandbox
size, or only your own keys or only built-in models. Its price is either a
fixed amount (US dollars per million tokens, or per hour running) or a
multiple of the provider's or catalog price: `0.8` is a 20% discount and `0`
counts usage at no cost, which suits a model you run yourself.

Rates are never edited. A change is a new rate from a later date, so past
costs keep the rate they used. Sandboxes have no catalog price, so they show
as not priced until you add a rate.

## Usage policies

A usage policy covers the organization, a person, an agent, a channel or
unattended work, measures cost or tokens over a day, week, month or in total,
and runs steps as usage grows:

- **Notify**: organization owners and admins get a notification, once per step
  and window.
- **Ask to continue**: the turn stops with a **Continue this turn** button for
  the person using the agent. Continuing applies to that session until the
  policy's window resets. Unattended work has no one to ask, so it stops.
- **Stop**: no more model calls until the limit is raised or the window resets.
  Every policy stops at 100% of its limit.

When several policies cover the same work, the most restrictive one wins. A
policy can count all usage, only built-in models, or only your own keys and
sandboxes. Days, weeks and months start at midnight in the organization's
timezone, set on the **Usage policies** tab (UTC until set).

In the API, usage policies are the `/v1/budgets` resource with a `policy`
object (`name`, `steps`, `source`); the price list is `/v1/cost-rates`, the
overview is `GET /v1/costs/summary`, and `POST /v1/sessions/{id}/resume`
answers an ask step for one session.
