---
type: Specification
title: What data plane v2 guarantees
description: "The promises agentd makes about writes, reads, side effects, spend and restores, per failure, and what it does not promise."
tags:
  - execution
  - data-plane-v2
  - durability
  - guarantees
---

# What data plane v2 guarantees

The contract of agentd (the shard, executor and edge processes of
[data plane v2](design.md)) in one place: what a caller can rely on once
agentd answered, what each failure costs, and what is not promised. The
mechanisms are in [design.md](design.md) ("Disaster recovery", "Executors",
"Placement"); how each promise is checked is in [testing.md](testing.md).
The public, operator-facing version is `docs/sre/agentd.md`. A change that
weakens a line here changes that page in the same PR.

## The commit point

A write is **committed** once a quorum of the shard's nodes has fsynced it:
two of three nodes, each in its own zone, in production; the only node on a
one-node shard (the Compose and Helm default). Followers fsync before they
acknowledge (`wal.rs`, group commit). Agentd answers a caller only after the
write is committed. The object store (S3, or a `file://` directory) is the
disaster copy, written after the commit: it is never the commit point, and
its own durability figures do not carry over to agentd's.

Three kinds of write must survive more than a quorum commit, because losing
them would break a promise below rather than just lose recent history: a
session's create, every step of a session move, and the `tool.started`
record of a call that must not run twice. These are **ship-first** writes
(`Node::ship_first`): answered only once every voter of a three-node shard
holds them, or, while a voter is down or more than 250 ms behind, once the
object store holds them.

## Writes, per failure

Three-node shard, nodes in three zones, restore points in an object store:

| Failure | Committed writes lost | Ship-first writes lost | Service |
| --- | --- | --- | --- |
| A node or a zone | None | None | A new leader within seconds; running turns continue |
| Two zones, the third node's disk left | What the survivor had not received yet, usually nothing | None | Down until `restore --survivor` rebuilds the shard (minutes) |
| Every disk of the shard (a region) | Writes not yet shipped: up to 10 s, never more than 30 s | Writes not yet shipped, same window | Rebuilt from the object store, in another region if needed |
| Region, rebuilt elsewhere | Also the bucket's replication lag (target 15 min) | Same | Restore in the second region |
| A bad release or a bad delete | Nothing, once restored to a point before it | None | Point-in-time restore of the shard or of one org |

One-node shard:

| Failure | Committed writes lost | Ship-first writes lost | Service |
| --- | --- | --- | --- |
| Process crash or restart | None | None | Back when the process is |
| The node's disk | Writes not yet shipped (10 s, at most 30 s) | None, if the store is elsewhere | Rebuilt from the store |
| The disk and a `file://` store on the same machine | Everything | Everything | None: the copy died with the original |

The 30 s bound is enforced: a leader whose shipping falls more than 30 s
behind stops admitting new turns until it catches up (`ship.rs`,
`BEHIND_LIMIT`).

## Reads and visibility

- Nobody sees an uncommitted write: not a stream, a listing, the control
  plane feed or another session.
- Readers see a write as soon as it is committed, before the object store
  has it. If every disk is then lost, a rebuild can lack a write someone
  already read, inside the shipping window. This is the price of not
  waiting for the store on every write; ship-first writes are the
  exceptions that do wait.
- Workspace file and SQL database reads are linearizable: any node answers,
  after catching up to the leader's read index (`api_files.rs`).
- A session's event stream resumes from its cursor (`since_id`) with no gap
  and no duplicate, across a stream node restart and a session move.
  Listings from a follower may trail the leader by the replication delay.
- A session's events are dense and in commit order; replicas agree byte for
  byte.

## Execution

- **One writer per session.** An executor writes under an epoch; a stale
  executor's writes are refused (`Epochs` model). A move never leaves two
  live owners, and a late message of an earlier move changes nothing
  (`Moves` model).
- **Turns outlive processes.** A turn continues after its executor dies or
  its shard's leader changes, from the session's log (`turn_resume.rs`).
  A turn whose holders keep dying without progress is sealed after a few
  epochs, never earlier.
- **A started call is never run twice.** A call that is not safe to repeat
  runs only after its `tool.started` is stored as a ship-first write, so it
  is known after any failure that leaves one disk or the bucket's copy of
  it, including a rebuild. Its idempotency key (session, message, call) is
  the same on every attempt. If its outcome was lost, the model is told the
  outcome is unknown; it is not rerun.
- **Every accepted input has one outcome**: a new turn, steering into the
  running turn, or a recorded rejection.

## The control plane boundary

- Feed records reach the control plane exactly once by id, in order,
  through delays, duplicates and leader changes (`Feed` model).
- Spend never passes the org's balance; with the control plane down, turns
  run on their allowance and then wait (`Grants` model).
- Config changes apply in order; a running turn keeps the agent revision it
  started on.
- A rebuilt shard is a new incarnation: nothing from the lost one (a late
  feed batch, a stale executor, an old move message) is accepted after it.

## Overload

Agentd refuses work rather than fall over: near its memory limit a shard
takes no new sessions, then answers new input with 429 and `Retry-After`;
admitted turns finish. A shard that stops committing, or falls 30 s behind
shipping, sheds new work the same way ([operations.md](operations.md)).

## Restores

Any moment inside the restore window (7 days by default,
`AGENTD_RESTORE_WINDOW_HOURS`) can be restored, for a whole shard or for
one org's sessions. Nothing a restore point refers to is deleted while the
point is in the window, and a restore takes back any segment it refers to
that was already marked for deletion (`restore_points.rs`).

## Not promised

- Surviving the loss of every disk of a shard without loss: that costs the
  shipping window.
- A region loss with less loss than the bucket's replication lag.
- Exactly-once effects for a tool that does not declare how it may be
  retried: idempotent tools run again after a crash and must make the
  repeat harmless (with the idempotency key).
- Latency. The SLO gauges ([operations.md](operations.md)) are budgets the
  load runs hold us to, not promises to a caller.
