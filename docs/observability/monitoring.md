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

## Timeline

**Timeline** shows recorded history instead of the current moment. Pick a range
from 15 minutes to 30 days; the server records a sample every 10 seconds and
keeps it permanently, so history exists before you open the page.

- **Sessions and agents**: open (not archived) sessions, defined agents, and
  agents with a turn running.
- **Working now**: turns running, averaged per bucket and split by phase, with a
  dashed line for the bucket's peak. Deploys are marked.
- **Turns** finished per bucket by outcome, and **Waiting for a worker**: the
  longest time a ready turn waited for a lease.
- **Tool calls**, **Model spend**, **Errors**, and **Model latency** (p95).

Click a bucket to see its numbers, or drag across any chart to select a range
and zoom into it. The side panel lists the busiest agents, failed turns by
cause, and failed tool calls by tool for the selection; **What failed** under
the Errors chart opens the same breakdown in a panel. Each summary tile
compares with the previous period of the same length.

When sessions run on data plane v2 cells, the charts include them, and an
**All / Workers / Cells** switch shows one execution path at a time. Workers are
sampled every 10 seconds; each cell reports once a minute, so cell lines move
in one-minute steps. An organization sees its own sessions on cells but nothing
about the cells themselves, which are shared.

The platform view adds worker capacity on the working chart, a **Fleet** chart
(server replicas, workers, cells, and cell nodes), the busiest cell node's CPU
and fullest node's memory against their limits, cell leader changes and session
moves, a load strip per worker (red above 90% of its slots), lease takeovers,
and worker joins and departures. It ranks organizations by ID only.

## Cells

The platform view has a **Cells** tab listing each data plane v2 cell from its
newest minute report: nodes (and how many answered the leader), sessions, turns
running, messages waiting, the busiest node's CPU and fullest node's memory, and
admit p99. A cell with no report for three minutes is marked silent. Select a
cell for its leader, log shipping p99, leader changes, moves, and charts of its
load, CPU and memory, and turns over the range. Cells need no metrics database:
the reports travel the cell's existing feed and are kept with the rest of the
history.

History from before recording started is rebuilt from turn records and drawn in
grey: it has running turns and outcomes, but no phase split, queue, or worker
load. Samples are taken every 10 seconds, so a spike shorter than that can be
missed; turn counts are exact.

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
bounded, filtered pages and actor detail, and `/timeline` returns recorded
history in buckets (`range` or `from`/`to`, optional `bucket` seconds) with
`/timeline/window` for a selection's busiest agents, failure causes, and failed
tools. `GET /api/v1/durable/monitoring/cells` lists cells with their history. The [API reference](/api/) owns the request and response contracts.

Monitoring complements [OpenTelemetry](/observability/opentelemetry/) and
[Braintrust](/observability/braintrust/), which provide trace exports and
provider-level diagnostics.
