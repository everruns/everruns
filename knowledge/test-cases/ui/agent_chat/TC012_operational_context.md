---
type: Test Case
title: "TC012: Operational page context"
description: "Verify operational context reflects visible bounded status rows and honors platform permissions and current filters."
tags: [ui, test-case, platform-chat, durable, monitoring]
---

# TC012: Operational page context

## Description

Verify Chat can explain visible operational state without exposing payloads or invoking operational actions.

## Preconditions

- Chat mode adopted; real chat model configured.
- Platform operator and separate tenant administrator without platform permission.
- Known worker, task, workflow, schedule and circuit-breaker fixture IDs; Monitoring enabled.

## Test Data

| Field | Value |
| --- | --- |
| Payload-only marker | omitted-payload-fixture |
| Row count | More than 50 where feasible |

## Steps

1. Open each Durable view and ask Chat to explain the current rows. Compare IDs,
   statuses, capacity, attempts and counts with the visible page, not a previous
   cached route. Change tabs, filters and search; repeat the read.
2. Put the marker in a task payload or error fixture. Neither it nor arbitrary
   payload/error bodies may enter the status summary. More than the row/text budget
   must report truncation and stay within the documented limits.
3. Navigate to workflow and schedule details. Verify identity, status and current
   execution rows. Ask to cancel, retry, trigger or delete through the draft tool:
   it must refuse; verify the fixture state and execution count remain unchanged.
4. On Monitoring switch execution, workers and actors, filter a phase, paginate and
   pause/replay. Chat context must match the displayed snapshot and identify paused
   state; hidden actor pages must not leak previous rows.
5. As the tenant administrator, direct-load global Durable routes and attempt the
   same reads. Without durable.view, global rows must be unavailable even if a prior
   operator loaded them. Loading, errors and revoked permission clear the projection.
6. Inspect personal Connections, MCP servers and AI clients, Features, Health,
   Slack and Payments: share public status metadata, never keys, auth methods or
   grants. Their immediate operations remain manual. Verify Adoption off, route/org
   switches, narrow pane, dark mode and reduced motion.

## Expected Result

Context follows displayed data, filters and snapshots. Bounded projections fail
closed under errors or denied platform permission, exclude protected payloads and
credentials, and grant no operational mutation authority.
