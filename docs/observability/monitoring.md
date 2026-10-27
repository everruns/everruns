---
title: Monitoring
description: Watch execution across thousands of sessions, map actor ownership to workers, and inspect durable leases as the fleet runs.
appliesTo: [platform, cloud]
sidebar:
  order: 1
---

Monitoring shows how the execution fleet is working, with aggregate population
counts and progressively narrower worker and actor detail. The overview keeps
agent names, prompts, and tool arguments out of the display.

## Organization and platform views

An organization administrator enables **Monitoring** in **Settings → Features**.
The feature uses the `monitoring` adoption flag and starts disabled. The
**Operational → Monitoring** destination then shows only the selected
organization's execution and actor ownership. Reading it requires session-read
permission; opting in does not grant additional permissions.

Platform operators open **System → Monitoring** for the cross-org
view. This view also exposes worker capacity, reported load, and heartbeat
freshness. Organization owners and admins cannot access this view unless they
also have platform access.

## Execution

**Flow** shows current sessions in runnable, model, tool, waiting, and setup
phases. Measured connections show committed event rates over the last minute.
Dashed connections illustrate the execution lifecycle. A turn can iterate
between model and tools many times before completing.

**Density** shows how that population changes over time. Each execution state
has its own intensity scale. Select a time bucket to inspect an earlier
population summary.

**Pause** freezes the displayed snapshot while observations continue to collect.
**Replay** retains up to fifteen minutes of population summaries collected while
this view is open. History starts at the first successful snapshot; it does not
invent earlier data. Historical worker maps are not inferred from today's
ownership. Return to **Live** for worker and actor inspection.

## Workers and actors

**Workers** groups execution by worker, with organization-to-worker ownership
mapping in the detail rail. Change group size to inspect individual workers or
larger cohorts. Worker load includes background activities and differs from
actor leases: a model phase may retain its lease while releasing a work slot.
Parked, unleased sessions remain visible in Execution.

**Actors** lists current session actors, with expired leases and the oldest
runnable work first. Select one to inspect its holder, fence, lease deadline,
last renewal, lease duration, inbox cursors, cancellation, ownership history,
and recent trace-step timing. A sequence difference is not a pending-message
count. Expiry is a lease observation and does not establish worker death.
Ownership history is retained for one day.

**Metrics** provides conventional event counts, rates, population trends, and
lease health. Overview summaries refresh every second. Failed refreshes are
marked stale rather than appearing healthy.

## API

Use `GET /api/v1/monitoring` for an enrolled organization's summary and
`GET /api/v1/durable/monitoring` for platform-wide monitoring. Both are read-only.
Use browser credentials or a full-access personal token; scoped personal tokens
remain denied. Organization adoption and read permissions still apply, and fleet
access still requires platform authority. The adjacent `/actors` endpoints provide
bounded, filtered pages and actor detail. The [API reference](/api/) owns the request and response contracts.

Monitoring complements [OpenTelemetry](/observability/opentelemetry/) and
[Braintrust](/observability/braintrust/), which provide trace exports and
provider-level diagnostics.
