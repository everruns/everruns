---
type: Playbook
title: Operating data plane v2 shards
description: "Inspecting a shard fleet, the per-shard SLO gauges, memory overload, and what to do about a lost leader, a stuck move and a drain."
tags:
  - execution
  - data-plane-v2
  - operations
---

# Operating data plane v2 shards

Part of [design.md](design.md) (step 10). The tools here ship in `crates/agentd`; the hosted load
fleet that exercises them is described in the SaaS repository (`knowledge/operations/dataplane-load.md`).

## Looking at a fleet

`agentd-testnode inspect` reads a fleet and changes nothing. It takes the topology file the nodes
use (`--topology`, or `AGENTD_TOPOLOGY`), adds the shards registered since, and asks every node.

| Command | Answers |
|---|---|
| `inspect shards` | Each shard's leader, how many nodes answer, applied and shipped log positions, snapshot age, sessions, running turns, queued inputs, unacknowledged feed records; per node how far it is behind, its memory, and `NO ANSWER` for a node that did not reply. `DRAINING` marks a draining shard. |
| `inspect session <id>` | Where the directory places the session, which shard and node answered, its placement and generation, turns done and whether one runs, who holds its claim and until when, how much of its history lives only in the object store, and its last events. |
| `inspect moves` | Every session that is frozen or prepared, next to what the directory says about it. |

`--json` prints the same as JSON. The answers come from `GET /shard/inspect` and
`GET /shard/sessions/{id}/inspect` on the nodes (`inspect.rs`).

## SLO gauges

Each node reports one-minute p99s as gauges next to its histograms, so a scraper that keeps
only gauges can chart the budgets (`prom::Window`):

| Gauge | Measures | Budget |
|---|---|---|
| `agentd_shard_slo_turn_p99_seconds` | Turns the shard finished, `turn.started` to the end of the turn, as each replica applied them | none; depends on the agent |
| `agentd_shard_slo_admit_p99_seconds` | Admitting a message through this node's gateway | 100 ms |
| `agentd_shard_slo_ship_lag_p99_seconds` | How long the oldest entry waited to reach the object store | 15 s |
| `agentd_shard_slo_ship_first_p99_seconds` | How long a write that must survive every lost disk waited for the object store before it was answered | 100 ms |

A minute with no samples reports 0. The worst minute in a longer period is the gauge's maximum.

## Memory overload

A node past its container's memory limit is killed, and catching up after the restart needs more
memory still, so a shard takes on less work before that (`pressure.rs`). Each node measures its
resident set against its limit (`--memory-limit-mb`, else the container's) and asks the shard's
other members for theirs every second; the worst node decides for the shard:

| Level | At | The shard |
|---|---|---|
| full | 85% (clears at 80%) | Gateways place no new sessions there; the rebalancer moves idle sessions to shards that are not full. |
| shedding | 92% (clears at 88%) | Refuses new sessions and new input: the gateway answers 429 with `Retry-After`. Turns already admitted keep running. |

Unloading events comes first and runs all the time (`memory_budget`); the levels are what happens
when unloading is not enough. The gauges are `agentd_shard_memory_pressure` (share of the limit),
`agentd_shard_overload_level` (0, 1, 2) and the `agentd_shard_shed_sessions_total` and `agentd_shard_shed_inputs_total`
counters; a level change is logged at WARN.

A shard that stays shedding has more sessions than its nodes hold: add a shard (new sessions go
there and the rebalancer evens out the rest) or give the nodes more memory. Lowering the memory
budget does not help: with too little room for events, every turn pages history from the object
store and throughput collapses (SaaS load run 43755911240).

Size the container at about 1.5 times the memory budget, not the budget plus a margin. The budget
counts events and the Raft log, but a snapshot copies the whole shard state and encodes it while
the log is at its longest, and the nodes of a shard snapshot at the same index: in the 75k feature
mix run that added about 3 GB to all three nodes of a shard at once, to 13.2 GB on a 9 GiB budget.
12Gi nodes with that budget were killed (SaaS load runs 44355318472 and 44383590806); 14Gi nodes
were not (44396952768).

## A shard that stops committing

A leader with entries it has not applied, and whose applied index has not moved for 5 s, sheds
like a shard short of memory: 429 with `Retry-After` for new sessions and new input, no new
placements, `agentd_shard_commit_stalled` 1 and `agentd_shard_overload_level` 2 on that node, a WARN when it
starts and stops. Without that, every request waits out its proposal deadline, clients retry, and
the waiting requests fill the nodes' memory (SaaS load run 43774871232: three shards stalled about
20 minutes each, and three nodes were killed).

The usual cause is Raft missing its deadlines on a host that is short of CPU: openraft gives an
append one heartbeat interval (300 ms) and a vote 1.5 s. Nodes serve Raft on their own listener
and threads (`--raft-port-offset`, deployed as port 7001 beside 7000; every member of a shard must
use the same offset), so client load does not delay it. A stall that lasts means the host is
oversubscribed: give the shards more CPU or move shards to another host.

## A node that rejoins far behind

A follower 2 048 or more committed entries short of what its leader last sent steps aside until
it is back within 256. While it does, it doesn't call elections, its `/health` answers 503, and it
answers every session request with 421, so gateways, streams and its peers use another member.
It keeps voting and taking the leader's entries. `agentd_shard_catching_up` is 1 on that node, and a WARN
marks the start and the end.

Any follower that has not seen a session answers 421 for it, so the gateway asks the next
member. Only the leader's "not found" is final.

Before this, a node that rejoined after a kill (SaaS load run 43803384547):

- answered "not found" for sessions created while it was down, so 6,790 sessions lost their first
  message;
- kept calling elections it couldn't win (term 244 by the end), each one unseating the leader.

It never caught up, because Raft's own listener refused snapshot chunks over 2 MB; it now takes
any size. `scripts/agentd/prove_rejoin_under_load.py` kills a follower under load and checks it
rejoins with no errors and no elections.

A node that stays at 1 for long is short of CPU or network to catch up. A node that keeps
returning to it is being killed again before it finishes.

## Leader loss

What happens by itself: a shard of three keeps working with two. The survivors elect a leader
within a few seconds; executors and gateways find it on their next call. Turns on executors keep
running, because their claims live in the shard, not on the node. A turn that ran on the lost node
itself resumes from the journal once its claim expires.

1. `inspect shards`. A shard that shows a leader and `2 of 3 nodes answer` is healthy and needs a
   replacement node, not an emergency.
2. Replace the node. When it comes back with its data directory, it catches up from the leader.
   When the data directory is gone, remove the member (`DELETE /v1/shards/{shard}/nodes/{node}`)
   and add a fresh one (`POST /v1/shards/{shard}/nodes`); it catches up from a snapshot.
3. `leader NONE` with fewer than two nodes answering means the shard lost its quorum and stops
   taking writes. Bring back a node that still has its data first. When two data
   directories are lost, stop the node that is left and rebuild the shard with
   `agentd-testnode restore --survivor <its data directory>` into fresh directories
   ([Disaster recovery](design.md#disaster-recovery)): it adds that disk's log to the object
   store's, so writes that had not shipped yet come back. Only with every directory gone,
   rebuild from the object store alone; then at most the last shipping lag of writes is lost.
   Either way the rebuild starts a new incarnation; never start the survivor in the old one.

Watch `agentd_shard_slo_ship_lag_p99_seconds` before taking a node down on purpose: with one node
down, losing a second would leave the third's disk, but losing both would leave only the
shipped log. `agentd_shard_ship_first_total{path="store"}` rising says writes are waiting on the
object store because a voter is behind. `agentd_store_puts_total{kind}` counts object writes.

## A stuck move

A move freezes the session in its shard, waits for the running turn to hand off at its next step,
copies the session to the target, repoints the directory and forwards. Every step is idempotent,
and a new leader runs the move again, so most stalls end on their own.

1. `inspect moves` lists the session with its placement and what the directory says.
2. `Frozen ... directory says <source>`: the source is waiting. `inspect session <id>` shows the
   claim. A claim that renews means a turn is still between steps; it hands off at the next one.
   A claim that no longer renews expires at the time shown, and the move then goes ahead. An
   executor that is alive but wedged can be restarted; its claim expires the same way.
3. `Frozen ... directory says <target>`: the ownership already moved; the source forwards once the
   target activates its copy. Check the target shard with `inspect shards`: a target without a
   leader holds the move until it has one again.
4. `Prepared` in the target with the directory still at the source: the source died after the
   copy. The prepared copy asks the directory after a short grace and either activates or drops
   itself.
5. A target that refuses the copy (full, draining, retired) makes the source abandon the move:
   the directory takes the session back at a higher generation and the source thaws it.

There is no manual abort. Fix what the move is waiting on: a claim, a leader, or a target.

## A drain

`POST /v1/shards/{shard}/drain` stops placing new sessions in the shard, and the rebalancer moves its
sessions out, running turns included, a share per round. `DELETE` on the same path stops the drain.

1. Start the drain and watch `inspect shards`: the shard shows `DRAINING` and its session count
   falls. `inspect moves` shows the moves in flight.
2. A session count that stops falling: look at `inspect moves` and follow "A stuck move".
3. At zero sessions the shard can retire (`DELETE /v1/shards/{shard}`). Its directory slots move to
   the other shards first.

An executor drains on SIGTERM: it stops pulling, hands every running epoch back at its next step,
and exits once none runs, or after 60 s when a turn cannot hand off (its shard does not answer);
that turn resumes elsewhere once its claim expires. Another executor resumes each turn from the journal. Deleting an
executor pod therefore costs the turns it ran one step each, not the turns.
