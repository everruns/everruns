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

- Organization monitoring is an adoption feature, disabled until opt-in. Every
  read enforces the resolved organization and the configured session-read policy.
- Cross-organization monitoring requires platform-user authority. Tenant roles
  and adoption never imply fleet access. Organization views expose only their
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
  sessions; the platform view ranks organizations by public ID only.

## Decisions

Agent semantics remain above `everruns-durable`; its dependency isolation guard
continues to apply. Monitoring queries the actor repository, small indexed event
envelopes, and the existing trace index. Ownership history is produced by actor
row transitions in the same transaction and expires through the existing
cluster-job mechanism.

Organization adoption and authorization are checked before the shared snapshot
cache, so revocation is observed on the next request. Cache entries and concurrent
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
read rules live in [the timeline repository](../../crates/server/src/storage/repositories/monitoring/timeline.rs)
and [the timeline API](../../crates/server/src/api/monitoring/timeline.rs).
Timeline endpoints follow the other Monitoring endpoints: scoped personal
access tokens are denied, browser sessions and full-access tokens are not.
Like the rest of Monitoring, Timeline has no CLI command.

The UI reuses Slate's page layout and canonical navigation. The default is Flow,
with Density available alongside it; Workers, Actors, and Metrics are separate
tabs. The implementation and sampling bounds live in
[the monitoring components](../../apps/ui/src/components/monitoring/monitoring-page.tsx).

## References

- [Durable execution](durable-execution-engine.md)
- [Public Monitoring guide](../../docs/observability/monitoring.md)
- [Threat model](../security/threat-model.md)
