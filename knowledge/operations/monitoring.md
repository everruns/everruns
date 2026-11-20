---
type: Specification
title: "Execution Monitoring"
description: "Organization and cross-organization monitoring of session actors, execution phases, worker ownership, and lease health."
tags:
  - everruns
  - operations
  - ui
---
# Execution Monitoring

## Abstract

Monitoring is a read-side view of the platform's session actors and committed
execution events. Its primary view explains a large fleet's movement through
model iterations, tools, waiting, and completion; conventional metrics remain
secondary. Agent names and conversation content are excluded from the overview.

## Requirements

- Organization monitoring is available to every organization. Each read
  enforces the resolved organization and the configured session-read policy.
- Cross-organization monitoring requires platform-user authority. Platform-user authority is independent of tenant roles. Organization views expose only their
  own ownership and omit global worker capacity and heartbeat telemetry.
- Live refreshes report observation time and stale status. Lease expiry, worker
  heartbeat freshness, current ownership, and reported work load are distinct.
- Population summaries do not allocate one browser element per session. Actor
  detail is separately paged and fetched only when needed. Truncated ownership
  detail is explicit; population and phase counts remain complete.
- Replay retains bounded population observations, not conversation content or
  fabricated historical worker assignments. It starts when the view opens.
- Reacquisition continues an actor under a new fence. It is never counted as a
  new turn. Renewal acknowledgements do not add ownership-history noise.

- Timeline shows recorded history over a chosen range (15 minutes to 30
  days): parallel sessions by phase with peaks, turns by outcome, wait for a
  worker, tool calls, spend, errors, and model latency; the platform view adds
  worker capacity and per-worker load. History is recorded server-side, so it
  exists before the page opens, and is never purged.
- Timeline drill-down names the busiest agents of the organization's own
  sessions; the platform view ranks organizations by public ID only. It also
  names the tools whose calls failed most.
- Timeline covers both execution paths on one time axis: workers (sampled every
  10 seconds) and data plane v2 cells (one report per cell per minute), plus
  open sessions, agents, and agents working. Numbers measured differently are
  shown side by side, never added in the API.
- Organization scope never shows cell nodes, CPU, memory, or elections: an
  organization shares cells with others. The platform view and its Cells tab
  show them: cells and nodes (pods), the fullest node's memory and busiest
  node's CPU against their limits, admit and shipping p99s, leader changes, and
  sessions moved between cells.

## Decisions

Agent semantics remain above `everruns-durable`; its dependency isolation guard
continues to apply. Monitoring queries the actor repository, small indexed event
envelopes, and the existing trace index. Ownership history is produced by actor
row transitions in the same transaction and expires through the existing
cluster-job mechanism.

Organization authorization is checked before reading the shared snapshot cache,
so revocation is observed on the next request. Cache entries and concurrent
reads are bounded. State classification and SQL bounds belong to
[the repository](../../crates/server/src/storage/repositories/monitoring/mod.rs);
authorization, response shapes, and caching belong to
[the API](../../crates/server/src/api/monitoring/mod.rs).

Timeline history lives in `monitoring_minutes`: one row per organization per
minute plus a platform row, because peaks do not add up across organizations.
A cluster-once job runs each minute, folds up to five 10-second population
samples into their minute, and recloses the last finished minutes from
committed events, lease history, and model generations, so late writes and
reconciled costs land. Firing a durable schedule every 10 seconds was rejected:
it would create a durable task per sample. Short peaks between samples are
missed; turn counters are exact. Organization rows exist only for active
minutes, so averages divide by the platform row's sample count. History from
before recording began is rebuilt a week per run from projected turn intervals
and committed events, marked `backfilled` (no phase split, queue, or worker
data). Reads aggregate minutes into the requested bucket. The sampling and
read rules live in [the timeline repository](../../crates/server/src/storage/repositories/monitoring/timeline/mod.rs)
and [the timeline API](../../crates/server/src/api/monitoring/timeline.rs).
Cells and the rest of the fleet are recorded without a metrics database, so the
history works on every install
([fleet history](../../crates/server/src/storage/repositories/monitoring/timeline/fleet.rs)).
Each cell's Raft leader proposes one report per finished minute; the report is
built at apply from replicated state plus the leader's node measurements and
travels the existing data plane feed, so a cell adds one record a minute and no
new connection. The control plane keeps the first report per cell and minute
(`monitoring_cell_minutes`) and adds its population to the platform row and the
rows of organizations with sessions on the cell. Finished turns and model calls
come from the feed's existing session and usage records, tallied per batch;
a batch redelivered after a lost acknowledgement can count twice, which
monitoring accepts. Open sessions and agents are counted once a minute into rows
that already exist; reads carry the last count forward, so idle organizations
cost no rows. A cell's Prometheus endpoint no longer labels series by agent:
series grew with every agent, and per-agent history comes from this record.
Timeline endpoints follow the other Monitoring endpoints: scoped personal
access tokens are denied, browser sessions and full-access tokens are not.
Like the rest of Monitoring, Timeline has no CLI command.

Per-node detail finer than a minute is optional and read, never stored: the
server queries a Prometheus-compatible store (VictoriaMetrics or Prometheus)
that scrapes the cells, for one cell's last hour at 10-second steps
([the reader](../../crates/server/src/domains/system/monitoring_detail.rs)).
PostgreSQL stays the record, so Monitoring works without the store, and a
missing or unreachable store is an answer that says why, not an error. The
store's address comes from the environment only, because an address settable
through the API would let platform users aim the server at internal hosts; pages
show its scheme and host, never credentials or a path. Cell names reach PromQL
only from a fixed character set. CloudWatch on AWS is served by alarms and
dashboards outside the app, not by this reader. The system doctor reports
whether minutes are still being recorded and whether the store answers.

An organization sees only the execution path it runs on: on a data plane, its
sessions on cells without the Workers and Actors tabs; otherwise the worker
view without cells. The platform view keeps both.

The UI reuses Slate's page layout and canonical navigation. The default is Flow,
with Density available alongside it; Workers, Actors, and Metrics are separate
tabs. The implementation and sampling bounds live in
[the monitoring components](../../apps/ui/src/components/monitoring/monitoring-page.tsx).

## References

- [Durable execution](durable-execution-engine.md)
- [Public Monitoring guide](../../docs/observability/monitoring.md)
- [Threat model](../security/threat-model.md)
