---
type: Specification
title: Session deletion
description: Deleting a session is a soft delete; what stays, what stops, and who still sees the session.
tags: [everruns, runtime-resources, sessions]
---

# Session deletion

Deleting a session never removes data. The rule for the whole platform is that
records are permanent: clutter is hidden, not purged. Users and organizations
are the exception, their deletion is a real erase that cascades through the
foreign keys.

## What a delete does

`Database::delete_session` (`crates/server/src/storage/repositories/session_delete.rs`)
sets `sessions.deleted_at` on the session and on every subagent session under
it, in one transaction. Nothing else about the rows changes: events, trace,
usage, files, SQL databases, storage values and secrets (encrypted values
included) all stay.

What stops with the session:

- its schedules are retired the same way deleting a schedule retires it;
- its unfinished tasks are asked to cancel;
- its sandboxes are marked deleted (the provider resource is disposable, the
  record is kept, see [Session Sandbox](session-sandbox.md));
- a turn that is running or queued is cancelled, after the marking commits.

## Who still sees a deleted session

Nobody a person or an agent can reach through the API. The org-scoped read
(`get_session`) and every list, count, facet, finder and channel-binding lookup
filter on `deleted_at IS NULL`, so a deleted session answers "not found", drops
out of lists and limits, and a new message on its channel thread starts a new
session. No new turn starts on it: the turn context loads through the same read.

Internal readers that go by id alone (`get_session_unscoped`,
`get_session_config`) and writers are deliberately not filtered. A worker that
was mid-turn when the delete landed finishes writing its events, and usage and
cost for that work are still recorded and still count toward agent, budget and
organization totals. Reports and monitoring aggregate over history, so they
keep counting deleted sessions too.

There is no restore surface. The data is there, a restore is clearing
`deleted_at`; exposing it is a product decision that has not been made.

## When the row really goes

Erasing a user or an organization deletes its session rows. Tables without a
foreign key to `sessions` (events, trace, generations) are cleaned by the
`sessions_unkeyed_delete_compat` trigger. A worker still attached to an
erased session gets "session not found" naming it and ends the turn quietly
(EVE-1235), as described in
[Durable execution engine](../operations/durable-execution-engine.md).
