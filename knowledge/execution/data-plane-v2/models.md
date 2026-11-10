---
type: Specification
title: Data plane v2 protocol models
description: TLA+ models of the cell protocols that are easiest to get wrong, each with a safe configuration that must pass and a bug configuration that must keep failing.
tags:
  - execution
  - data-plane-v2
  - testing
  - tla
---

# Data plane v2 protocol models

Part of [design.md](design.md), plan step 0. The models live in
[`tests/models/data-plane-v2/`](../../../tests/models/data-plane-v2/) and
[`scripts/check-tla-models.sh`](../../../scripts/check-tla-models.sh) runs
them with TLC (pinned release, checksum verified); the `Protocol models
(TLA+)` workflow runs it on every change to them.

Each model is small on purpose: three cells, a handful of generations,
grants or entries. Messages are never removed from the network set, so every
message can arrive late and more than once. Each model also has a **bug
configuration** that switches off one rule of the protocol. It reproduces a
real flaw (in funout or in an earlier draft of this design) and must keep
failing: a bug configuration that starts passing means the model no longer
checks what it claims to.

| Model | Safe configuration checks | Bug configuration reproduces |
| --- | --- | --- |
| `Moves` | At most one cell can run a unit; a live copy is the one the directory points at; the directory's generation never goes backwards. Includes abandoned moves and dropping stale prepared copies. | funout at `8ce744b` (`Import` accepts any unit it doesn't hold and runs it, `Place` overwrites, `Forward` is unconditional): a delayed import revives a stale copy, two live owners. |
| `Grants` | Providers never bill more than the org's balance; the ledger never authorizes more; a closed grant's settled usage is everything spent from it. Includes unresolved operations, delayed and duplicated settlements, and a lost cell. | A grant released on expiry at the usage heard so far: real spend passes the balance (the "$100 grant, $160 spent" case). |
| `Feed` | The control plane stores exactly the log's records, in order, each once, through delayed, duplicated and reordered batches. | Records keyed by Raft index alone: the second record of a batched entry is silently skipped. |
| `Epochs` | The journal takes writes from one epoch at a time and never from an older one; the poison guard seals a turn only after K holder deaths without accepted progress, and never allows more. Includes executors restarting under the same name and a proposer clock that moves back. | Three: writes fenced by holder name (a restarted executor's earlier incarnation writes behind the new epoch: the journal goes backwards); the guard ignoring progress (core before the fix: a turn that advances in every epoch is sealed); progress accepted from any epoch (a late report keeps a dying turn alive past K). |

## What the models changed in the design

Writing `Moves` found a hole in the first version of placement generations:
the destination's imported copy and the source's departing copy were one
"frozen" state, so a late duplicate activation could wake a copy that had
already started leaving. The design now has distinct **prepared** and
**leaving** states, activation only from prepared, and an abandoned move
that takes the unit back through the directory at a higher generation.

## Next models

`Epochs` came with plan step 5. It records one accepted limit: an epoch
that dies after its second step but before its progress report lands
counts as a death without progress, a window of one round trip.

Plan step 0 also names turn revision retention and recovery incarnations;
step 1 adds the committed event batch (a move while a batch is pending, an
executor killed before commit). Each is written before the code it covers.
