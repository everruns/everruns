---
type: Playbook
title: Operating data plane v2 cells
description: "Inspecting a cell fleet, the per-cell SLO gauges, memory overload, and what to do about a lost leader, a stuck move and a drain."
tags:
  - execution
  - data-plane-v2
  - operations
---

# Operating data plane v2 cells

Part of [design.md](design.md) (step 10). The tools here ship in `crates/cell`; the hosted load
fleet that exercises them is described in the SaaS repository (`knowledge/operations/dataplane-load.md`).

## Looking at a fleet

`cell-testnode inspect` reads a fleet and changes nothing. It takes the topology file the nodes
use (`--topology`, or `CELL_TOPOLOGY`), adds the cells registered since, and asks every node.

| Command | Answers |
|---|---|
| `inspect cells` | Each cell's leader, how many nodes answer, applied and shipped log positions, snapshot age, sessions, running turns, queued inputs, unacknowledged feed records; per node how far it is behind, its memory, and `NO ANSWER` for a node that did not reply. `DRAINING` marks a draining cell. |
| `inspect session <id>` | Where the directory places the session, which cell and node answered, its placement and generation, turns done and whether one runs, who holds its claim and until when, how much of its history lives only in the object store, and its last events. |
| `inspect moves` | Every session that is frozen or prepared, next to what the directory says about it. |

`--json` prints the same as JSON. The answers come from `GET /cell/inspect` and
`GET /cell/sessions/{id}/inspect` on the nodes (`inspect.rs`).

## SLO gauges

Each node reports one-minute p99s as gauges next to its histograms, so a scraper that keeps
only gauges can chart the budgets (`prom::Window`):

| Gauge | Measures | Budget |
|---|---|---|
| `cell_slo_turn_p99_seconds` | Turns the cell finished, `turn.started` to the end of the turn, as each replica applied them | none; depends on the agent |
| `cell_slo_admit_p99_seconds` | Admitting a message through this node's gateway | 100 ms |
| `cell_slo_ship_lag_p99_seconds` | How long the oldest entry waited to reach the object store | 2 s |
| `cell_slo_ship_first_p99_seconds` | How long a ship-first write waited for the object store before it was answered | 100 ms |

A minute with no samples reports 0. The worst minute in a longer period is the gauge's maximum.

## Memory overload

A node past its container's memory limit is killed, and catching up after the restart needs more
memory still, so a cell takes on less work before that (`pressure.rs`). Each node measures its
resident set against its limit (`--memory-limit-mb`, else the container's) and asks the cell's
other members for theirs every second; the worst node decides for the cell:

| Level | At | The cell |
|---|---|---|
| full | 85% (clears at 80%) | Gateways place no new sessions there; the rebalancer moves idle sessions to cells that are not full. |
| shedding | 92% (clears at 88%) | Refuses new sessions and new input: the gateway answers 429 with `Retry-After`. Turns already admitted keep running. |

Unloading events comes first and runs all the time (`memory_budget`); the levels are what happens
when unloading is not enough. The gauges are `cell_memory_pressure` (share of the limit),
`cell_overload_level` (0, 1, 2) and the `cell_shed_sessions_total` and `cell_shed_inputs_total`
counters; a level change is logged at WARN.

A cell that stays shedding has more sessions than its nodes hold: add a cell (new sessions go
there and the rebalancer evens out the rest) or give the nodes more memory. Lowering the memory
budget does not help: with too little room for events, every turn pages history from the object
store and throughput collapses (SaaS load run 43755911240).

## A cell that stops committing

A leader with entries it has not applied, and whose applied index has not moved for 5 s, sheds
like a cell short of memory: 429 with `Retry-After` for new sessions and new input, no new
placements, `cell_commit_stalled` 1 and `cell_overload_level` 2 on that node, a WARN when it
starts and stops. Without that, every request waits out its proposal deadline, clients retry, and
the waiting requests fill the nodes' memory (SaaS load run 43774871232: three cells stalled about
20 minutes each, and three nodes were killed).

The usual cause is Raft missing its deadlines on a host that is short of CPU: openraft gives an
append one heartbeat interval (300 ms) and a vote 1.5 s. Nodes serve Raft on their own listener
and threads (`--raft-port-offset`, deployed as port 7001 beside 7000; every member of a cell must
use the same offset), so client load does not delay it. A stall that lasts means the host is
oversubscribed: give the cells more CPU or move cells to another host.

## Leader loss

What happens by itself: a cell of three keeps working with two. The survivors elect a leader
within a few seconds; executors and gateways find it on their next call. Turns on executors keep
running, because their claims live in the cell, not on the node. A turn that ran on the lost node
itself resumes from the journal once its claim expires.

1. `inspect cells`. A cell that shows a leader and `2 of 3 nodes answer` is healthy and needs a
   replacement node, not an emergency.
2. Replace the node. When it comes back with its data directory, it catches up from the leader.
   When the data directory is gone, remove the member (`DELETE /v1/cells/{cell}/nodes/{node}`)
   and add a fresh one (`POST /v1/cells/{cell}/nodes`); it catches up from a snapshot.
3. `leader NONE` with fewer than two nodes answering means the cell lost its quorum and stops
   taking writes. Bring back a node that still has its data first. When two or more data
   directories are lost, rebuild the cell from the object store with `cell-testnode restore`
   ([Disaster recovery](design.md#disaster-recovery)); the rebuild starts a new incarnation, and
   at most the last shipping lag of writes is lost.

Watch `cell_slo_ship_lag_p99_seconds` before taking a node down on purpose: what a lost cell
would lose is bounded by the shipping lag, not by the node count.

## A stuck move

A move freezes the session in its cell, waits for the running turn to hand off at its next step,
copies the session to the target, repoints the directory and forwards. Every step is idempotent,
and a new leader runs the move again, so most stalls end on their own.

1. `inspect moves` lists the session with its placement and what the directory says.
2. `Frozen ... directory says <source>`: the source is waiting. `inspect session <id>` shows the
   claim. A claim that renews means a turn is still between steps; it hands off at the next one.
   A claim that no longer renews expires at the time shown, and the move then goes ahead. An
   executor that is alive but wedged can be restarted; its claim expires the same way.
3. `Frozen ... directory says <target>`: the ownership already moved; the source forwards once the
   target activates its copy. Check the target cell with `inspect cells`: a target without a
   leader holds the move until it has one again.
4. `Prepared` in the target with the directory still at the source: the source died after the
   copy. The prepared copy asks the directory after a short grace and either activates or drops
   itself.
5. A target that refuses the copy (full, draining, retired) makes the source abandon the move:
   the directory takes the session back at a higher generation and the source thaws it.

There is no manual abort. Fix what the move is waiting on: a claim, a leader, or a target.

## A drain

`POST /v1/cells/{cell}/drain` stops placing new sessions in the cell, and the rebalancer moves its
sessions out, running turns included, a share per round. `DELETE` on the same path stops the drain.

1. Start the drain and watch `inspect cells`: the cell shows `DRAINING` and its session count
   falls. `inspect moves` shows the moves in flight.
2. A session count that stops falling: look at `inspect moves` and follow "A stuck move".
3. At zero sessions the cell can retire (`DELETE /v1/cells/{cell}`). Its directory shards move to
   the other cells first.

An executor drains on SIGTERM: it stops pulling, hands every running epoch back at its next step,
and exits once none runs, or after 60 s when a turn cannot hand off (its cell does not answer);
that turn resumes elsewhere once its claim expires. Another executor resumes each turn from the journal. Deleting an
executor pod therefore costs the turns it ran one step each, not the turns.
