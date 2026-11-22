---
type: Specification
title: How data plane v2 is tested
description: "The layers that check agentd's guarantees today, which check covers which promise, the rules every test follows, and the gaps still open."
tags:
  - execution
  - data-plane-v2
  - testing
---

# How data plane v2 is tested

Every promise in [guarantees.md](guarantees.md) has a check that fails when
the promise breaks. This page says what runs today, what each layer can and
cannot catch, and what is still missing. The target design (deterministic
simulation, CI tiers, budgets) is in [design.md](design.md), "Testing"; this
page is the state of it.

## Rules

- **A test double must not be kinder than the real thing.** Where a
  guarantee rests on what the object store does (a conditional create, a
  listing after a write, timestamps on a rewrite), the fake's behavior is
  checked against the real store by `agentd-testnode store-check`, which
  every load run runs against the production-class bucket. A storage bug
  that only the real store has is the failure this rule exists for.
- **Prove the window exists before testing what survives it.** A durability
  test first shows the state it claims to survive: `durability.rs` slows
  the store (`AGENTD_STORE_THROTTLE_MS`) and asserts the bucket does not
  yet have the write, then that the rebuild does. A test that passes
  because the store was fast proves nothing.
- **Every model has a bug configuration that must keep failing**
  ([models.md](models.md)). A bug configuration that starts passing means
  the model stopped checking what it claims.
- **Side effects are counted, not inferred.** Fleet tests run a tool that
  appends to a ledger file (`AGENTD_LEDGER`), so "ran exactly once" is a
  line count, compared with the `tool.started` events after the failure.
- **No `#[ignore]`.** What cannot run on every PR runs weekly or by hand
  with its own workflow.

## Layers

| Layer | Where | Runs | Catches | Cannot catch |
| --- | --- | --- | --- | --- |
| Storage proofs | `crates/agentd/src/wal_tests.rs`, `store.rs`, `restore_points.rs`, `restore.rs` tests | Every PR | Lost acknowledged entries after a crash at named points and torn tails (200 seeds, each replays exactly); retention and restore edge cases | Faults outside the write-ahead log; real store behavior |
| Protocol models | `tests/models/data-plane-v2/` (TLA+, TLC) | Every change to a model | Interleavings of moves, epochs, grants, the feed, durability that no run is likely to hit | Drift between the model and the code |
| Fleet tests | `crates/agentd-testkit/tests/` | Every PR touching `crates/agentd*` (`ci.yml`) | Real shards, executors and edge in one process, each node in its own runtime so it can be killed; fake control plane; simulated model | Network partitions, slow or failing disks, process-level crashes with unsynced pages |
| Store check | `agentd-testnode store-check` (`store_check.rs`) | Every load run (SaaS `dataplane-load.yml`), and by hand on a new bucket | The real store disagreeing with what restore and collection assume | Store faults (5xx, throttling) |
| Recovery drill | `scripts/agentd/drill.py`, `agentd-drill.yml` | Weekly, and by hand at full size | A whole shard lost with SIGKILL under load, rebuilt from the store: every session finishes, no tool ran twice, rebuild time and shipping lag | The survivor path (the drill rebuilds from the store alone) |
| Load runs | SaaS `dataplane-load.yml` on EKS | By hand before each ramp step | The 75k fleet with the production feature mix, a node and executors killed mid-run; errors, OOM kills, SLO gauges, object store writes by kind | Anything rarer than a run surfaces |

## Which check covers which promise

| Guarantee | Checks |
| --- | --- |
| A committed write survives a crash | `wal_tests.rs` (seeded crash points); `executors.rs` `a_turn_survives_its_shards_leader_dying` |
| Ship-first writes survive two lost zones | `Durability` model; `durability.rs` (store provably behind); `recovery.rs` two-zone tests, including a survivor that lagged |
| Losses without any disk stay inside the window | Recovery drill (resends what the rebuild lost, counts it); `ship.rs` tests; ship-lag gauge in load runs |
| One writer per session | `Epochs` and `Moves` models; `qualification.rs` `a_stale_executor_is_fenced_out`, `old_messages_of_earlier_moves_change_nothing` |
| A started call never runs twice | `qualification.rs` `a_shard_restart_after_an_external_effect_runs_it_once`; `recovery.rs`; drill (ledger count) |
| Turns outlive executors and leaders | `executors.rs` (killed executor, leader death, Raft on its own threads) |
| No uncommitted event seen; streams have no gap | `stream.rs` (stale publisher, restarted stream node, move between replay and live) |
| Feed exactly once | `Feed` model; `control_plane.rs` `a_delayed_feed_arrives_once_across_a_leader_change` |
| Spend within balance | `Grants` model; `control_plane.rs` (lost acknowledgements, control plane down, expired grants); `allowance.rs` |
| Config order and revisions | `control_plane.rs` (rollback, mid-turn change) |
| Overload sheds instead of failing | `executors.rs` memory and stopped-commit tests; load runs (0 OOM kills gate) |
| Restores | `recovery.rs` point-in-time and one-org; `restore.rs` and `restore_points.rs` tests; store check |
| Tenancy | `edge.rs` `no_token_reaches_another_orgs_session` |

## Gaps

Known, in the order we would close them:

1. **Deterministic simulation of the shard.** Only the write-ahead log runs
   under seeds. The shard, its Raft wiring and the executors run on the
   real scheduler, so a rare interleaving found once may not replay. The
   design calls for madsim or turmoil with fault hooks at the model's
   interleaving points.
2. **Network faults.** Fleet tests kill nodes but never partition or delay
   them; the drill and load runs kill pods only.
3. **Disk faults beyond the log.** Full disk, fsync errors and slow fsync
   on snapshots and segments are untested.
4. **Store faults.** The store is slowed in tests, never made to fail
   (5xx, throttling, a lost response after a successful write).
5. **Model to code.** Nothing checks that the code's transitions are the
   models' transitions (trace checking).
6. **The survivor path at scale.** `restore --survivor` is covered by fleet
   tests; the weekly drill still rebuilds from the store alone.
7. **Cross-region restore.** Bucket replication and the second-region drill
   wait on the production bucket setup.
