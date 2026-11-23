---
type: Proposal
title: Retiring data plane v1
description: "What must hold before the worker data plane (v1) is removed, the order it goes in, what stays, and who picks the date."
tags:
  - execution
  - data-plane-v2
  - operations
---

# Retiring data plane v1

Part of [design.md](design.md). Data plane v2 (agentd) is the default for new organizations in
self-hosted installs (`--data-plane`, `EVERRUNS_DATA_PLANE`) and on the hosted demo. v1, today's
workers, stays selectable until it is retired. The owner picks the retirement date; this concept
says what has to be true first and what the removal touches, so the date is a decision, not an
investigation.

## Gates

All of these hold before the date is set:

- **Scale.** A 75 000-agent run on the live fleet passes the load gate in the SaaS repository
  (`knowledge/operations/load-testing.md`), with zero errors and no out-of-memory kills.
- **Parity.** Every capability in the capability matrix runs on a shard as a native capability or
  through an injected server ([native vs injected](platform-on-engine.md)); nothing still needs the
  worker turn loop.
- **Every hosted organization has moved.** `everruns-dataplane move-orgs` (or the per-org walk) has
  moved every organization with sessions, archived ones included, and the moves table shows none
  frozen or failed.
- **Self-hosted installs had a release to move.** At least one release ships `dataplane move-orgs`
  and its upgrade notes ([Data plane (agentd)](../../../docs/sre/agentd.md#move-organizations-from-v1))
  before the release that removes v1, and the changelog for that earlier release says v1 is going.
- **Recovery is proven on v2.** The disaster-recovery drill (shard rebuild and restore point
  restore) passes on the release that removes v1 ([operations.md](operations.md)).

## What goes

In this order, one release each, so a problem in one step does not hide behind the next:

1. **New organizations can no longer pick v1.** `--data-plane v1` and `EVERRUNS_DATA_PLANE=v1`
   are refused with a message that points at the upgrade notes. Existing v1 organizations keep
   running.
2. **Startup moves what is left.** A server that finds v1 organizations with sessions refuses to
   start and names the command that moves them, rather than moving them on its own: a move is
   something the operator watches.
3. **The worker turn loop is removed**: the v1 session execution path, the worker turn admission
   and queue, and the `DATA_PLANE_REPLICAS=0` and `DATA_PLANE_PULL_POLICY` switches in Compose and
   the chart.

## What stays

- **Session records in Postgres.** Moved sessions keep their v1 rows (soft state, never purged),
  so history, cost and audit reads keep working.
- **The control plane's worker commands behind injected servers.** Platform commands, credentials
  and the person's MCP servers answer shard tool calls through `domains/data_plane/calls.rs`; they
  are control plane code, not the v1 data plane.
- **The `/dp/v1` contract**, which is the control plane and data plane protocol's version, not
  data plane v1.

## Open

- Whether the hosted demo removes v1 before self-hosted releases do, or both go in one release.
- How long step 1 runs before step 2 (one release cycle is the default).
