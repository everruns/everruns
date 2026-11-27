---
type: Design
title: Segment packs
description: "One object per flush pass for every session's history segments on a shard, instead of two objects per session per flush plus a retirement mark per replaced segment."
tags:
  - execution
  - data-plane-v2
  - object-store
  - cost
---

# Segment packs

Status: built, 2026-11-27. Part of [design.md](design.md) ("History in S3").
Code: `crates/agentd/src/packs.rs` (pack ids, the builder, sidecars, pins and
the sweep), `store.rs` (`SegRef`, ranged reads), `sm.rs` (`Shared::flush`),
`node_restore.rs` (`store_events`), `restore_points.rs` (sidecars beside
snapshot copies, the legacy retirement marks) and `restore.rs` (pins).

What was built differs from the proposal below in three places:

- **Legacy retirement stays for legacy segments.** A compaction that replaces
  segments written before packs still marks them retired, so the collector
  deletes them as before; it no longer LISTs the session's prefix to find
  them, but takes them from the state. Pack entries get no marks. The
  retirement code goes once no state names a legacy segment.
- **Sidecars come from the snapshot copy.** `put_snapshot` scans the copy's
  state JSON for pack ids and writes the sidecar before the copy, so every
  writer of a copy (a leader's archiver, a rebuild) writes one. The sweep
  needs no internal API: a pack an old state still names is in every kept
  snapshot since, and a restore pins what it brought back
  (`packs-pinned/`) for a window plus the grace. A snapshot without a
  sidecar that is newer than the oldest pack stops the sweep.
- **No repack yet.** Dead space costs storage, not requests; it comes after
  the request count is measured at goal pace.

## Why

Object-store requests are the largest cost of the data plane. In the 75k gate,
writes cost $7.0e-5 per agent-hour against $3.3e-5 for compute (`goal.toml`,
`[cost]`), at 1.34 PUTs per turn on the feature mix. The C5 bar is 1.5.

On the coding workload (75k verifier G1, `memory_bench.py --coding`, Mac mini,
2026-11-26), a turn makes **5.7 PUTs**:

| Kind | PUTs per turn | What writes it |
| --- | --- | --- |
| `segment` | 3.8 | `put_segment`: two objects per flush of one session |
| `retired` | 1.0 | `retire_segments`: one empty mark per segment a compaction replaces |
| `log` | 0.9 | the shipped Raft log and ship-first writes (`ship.rs`) |
| `snapshot` | ~0 | archived snapshots |

At goal pace (75k agents, one turn every 6 minutes, about 208 turns/s), that is
about 1,200 PUTs/s, roughly $21/h or $15k a month in PUTs alone, compared with
about $5/h at 1.34.

### Why about 4 segment objects per coding turn

- `put_segment` writes **two objects per flush**: `<from>-<to>.messages.json`
  (the context events, `is_context`) and `<from>-<to>.json` (every event).
- A running turn is flushed once its unstored tail reaches `FLUSH_BUSY_AT` (64)
  events. A coding turn has many model calls, steps and tool lifecycle events,
  so it passes 64 at least once mid-turn. Its remainder is flushed after
  `FLUSH_IDLE_AFTER` (60 s) of quiet. That makes about two flushes per turn,
  and so about four objects.
- Compaction (`COMPACT_AT` = 16 segments, tiered) adds a rewrite pair now and
  then. Under memory pressure the store loop passes `idle_for = 0`, which
  flushes every session with a tail on each 500 ms pass and adds segments.

### The per-turn `retired` write

`store_events` records a compaction, then `prune_segments` LISTs the session's
prefix and writes one empty object per segment the compaction covers,
`retired/<hour>/<session>/<from>-<to>.json`. The collector (`collect_segments`,
on the directory shard's leader every `COLLECT_EVERY`) lists the expired hours
and deletes those segments. Every flushed segment is eventually covered by a
compaction, and tiered outputs are covered again about log(n) times, so this
comes to about one mark per turn, plus one LIST per compaction. The marks exist
only so that the collector does not delete a segment that a restore point
inside the window still names (`unretire` takes them back on a restore).

## Design

### One pack per flush pass

The leader's store loop already gathers up to 512 sessions per pass
(`Shared::flush`). Instead of `put_segment` per session, the pass builds **one
pack**: every session's new segment from that pass, compactions included, in
one object.

```
packs/<shard>/<incarnation>/<pack-id>.pack
  header: [ { session, branch, from, to, events: (offset, len), context: (offset, len) } ... ]
  body:   for each entry, the segment's events, then its context events
```

- `pack-id` is unique per writer, for example the Raft index the pass read at
  plus the node id. Packs are immutable. A retried PUT of the same bytes is
  harmless, and a new leader never reuses an id.
- A pack is capped (for example 8 MiB or 512 entries). A pass that exceeds the
  cap writes several packs, still a handful per pass rather than one or two per
  session.
- The **messages-only companion is folded in** as the entry's `context` range.
  It costs no PUT of its own. The executor's sparse reads of earlier turns stay
  as cheap as now.
- Request count: at most one PUT per shard per 500 ms pass while there is
  anything to store. At goal pace that is single PUTs per second, where today's
  segment writes come to about 790 PUTs/s.

### What a session records

`SegRef` (in `SessionState::segments`, replicated, carried in snapshots) gains
the pack location:

```rust
pub struct SegRef {
    pub from: i32,
    pub to: i32,
    pub branch: u64,
    // new: where the segment's bytes are
    pub pack: Option<PackId>,  // None: a legacy per-session object
    pub events: (u32, u32),    // offset, len within the pack
    pub context: (u32, u32),
}
```

`Command::Stored` records the entry after the pack's PUT succeeded, as it does
today for one segment. A pack nobody recorded (the leader lost leadership
between the PUT and the commit) is garbage. The sweep below deletes it.

### Reads

- `load` / `read_range`: one **ranged GET** per segment
  (`get_range(pack, events)`). That is the same GET count as today, and GETs
  are a tenth of a PUT's price. When several of a session's segments sit in
  one pack, contiguous ranges are merged into one GET.
- `load_context`: a ranged GET of the entry's `context` range.
- The executor's immutable cache (`get_immutable`) keys on
  `(pack, offset, len)`. Packs are immutable, so nothing is ever invalidated.
- Legacy segments (`pack: None`) are read as today, so the change needs no
  migration. They disappear as compaction rewrites them into packs.
- `segments_of` (a LIST of the session prefix) is no longer needed on the read
  path. The state's segment list is authoritative, and a missing range is an
  error to report, not a reason to re-list.

### Garbage collection: mark and sweep instead of retirement marks

Per-segment marks do not work for shared objects: a pack holds live and
retired entries of many sessions. Replace them with a sweep:

1. **Live set.** A pack is live while any of the following still names it:
   - a session in any shard's current state;
   - a restore point inside the restore window, which is an archived snapshot
     plus the shipped log after it. Each archived snapshot gets a small sidecar
     listing the pack ids its sessions reference. It is written with the
     snapshot copy, so it adds one PUT per snapshot, not per turn.
2. **Sweep.** On its existing `COLLECT_EVERY` tick, the directory shard's
   leader lists `packs/` and deletes a pack only if all of these hold:
   - it is older than the restore window plus a grace period;
   - no shard reports it live; each shard serves its live pack set over the
     internal API, the same way the collector already reads restore points;
   - no snapshot sidecar inside the window names it.
3. **Removed.** `retire_segments`, `prune_segments`' LIST and per-mark writes,
   `collect_segments` and `unretire`. A restored state names its packs, so a
   restore needs no un-retiring: anything a restore point inside the window
   could name is live by construction.

Request cost: the per-turn `retired` PUT (about 1.0 per turn) and the
per-compaction LIST go away. The sweep costs one LIST page per 1,000 packs per
collect plus DELETEs, which are free on S3.

**Dead space.** A pack stays until its last referencing entry is retired. A
session that goes quiet pins its entries, and with them every other session's
dead bytes in the same pack. Bound it with a **repack**: when the sweep finds a
pack older than the window whose live entries are under a quarter of its bytes,
the owning shard's leader rewrites those live entries into its next pack, as if
they were compactions. That is one more entry in a pack it writes anyway, with
no extra PUT. Storage costs about $0.023 per GB-month, so the repack threshold
can be lazy.

### Restore

Restore (`restore.rs`) rebuilds state from an archived snapshot plus the
shipped log. The segment lists it restores name packs. The packs are immutable
and, being referenced by a restore point inside the window, were not swept.
Nothing else changes. An org restore that brings back an older state works the
same way.

### Moves

A move hands over the session's state with its segment list, history left in
the bucket (`move_image`). Pack keys do not depend on which shard owns the
session, so the target reads the same ranges. The source shard stops counting
the session as soon as the move commits. The target counts it from its own
state, so the sweep's union of every shard's live set keeps the packs. No copy
is needed and no ownership is transferred.

### Compaction

Unchanged in policy (tiered, `compaction_range`), but a compaction's output is
one more entry in the pass's pack, not its own pair of objects, and it writes no
retirement marks. The old entries simply stop being referenced.

### Ship-first and the shipped log (`log`, 0.9 per turn)

This is not part of the pack change, but it belongs to the same budget:

- Ship-first writes are batched per shard at `FLUSH_EVERY` (100 ms), so their
  per-turn cost falls as load rises.
- At goal pace a shard ships at most about 10 objects/s for them, plus one per
  `SHIP_EVERY`.

Measure it at a realistic rate before changing anything. The single-shard bench
runs at low turn rates, which overstates it.

## Expected effect

| Kind | Today (coding) | With packs, at goal pace |
| --- | --- | --- |
| segment | 3.8 per turn | ~0.01 per turn (single PUTs per shard per second) |
| retired | 1.0 per turn | 0 (sweep: LIST pages, free DELETEs) |
| log | 0.9 per turn | unchanged, falls with batching |

That puts coding turns well under the C5 bar of 1.5 and the 75k gate's 1.34,
probably under 1 at goal pace.

## Tests to write with it

- A flush pass of N sessions writes one pack: count PUTs by kind, which
  `store::puts()` already does.
- Every read path returns identical events from a pack and from legacy objects:
  `load`, `load_context`, `image`, restore, a move then a read on the target.
- A leader that dies between the PUT and `Stored` leaves an unreferenced pack,
  which the sweep deletes after the window and never sooner.
- A restore to a point inside the window reads every segment it names after a
  sweep has run.
- Repack: a pack under the live-share threshold has its live entries rewritten,
  and is then deleted once unreferenced.
- `memory_bench.py --coding` reports `segment` + `retired` under 0.1 PUTs per
  turn, a new floor for C5 on the coding workload.
