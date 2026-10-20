---
type: Specification
title: Dreaming
description: Background evidence-grounded consolidation into existing Memory file surfaces, independent of foreground compaction.
tags: [everruns, memory, dreaming, framework]
---

# Dreaming

Dreaming is opt-in background maintenance over completed experience. It neither
rewrites foreground history nor changes compaction checkpoints. Configuration
and validation belong to the [neutral contract](../../crates/contracts/src/runtime/dreaming.rs);
shared execution belongs to the [portable runner](../../crates/core/src/engine/dreaming/runner.rs)
and [host orchestration](../../crates/core/src/host/dreaming.rs).

## Boundaries

- A writable Memory identity is host-authorized. A pathname alone is insufficient.
- User-private evidence cannot be promoted into broader shared Memory.
- Inputs, existing notes, generated output, retries, and spending are bounded.
- The model has no tools. Notes must cite selected evidence and distinguish
  inference from observation. Citation validation establishes provenance, not entailment.
- Memory writes use operation-specific artifacts, preserving human edits.
  Reconciliation is expressed in new notes; global in-place rewriting is not implemented.
- Proposals remain in protected maintenance state, outside active Memory.
- Scope-level quotas are reserved atomically before a billable call. Unknown
  pricing fails closed when a dollar budget is requested. Reservations remain
  charged after uncertain failures.
- Output is persisted before file writes. Recovery repeats idempotent writes;
  a call lost before output persistence can be billed again.
- Payload-free generation observations feed existing usage/budget listeners; Memory evidence and output never enter those events.
- Maintenance claims are separate from foreground actor leases. Only bounded
  activity metadata may append outside a foreground epoch; task effects remain fenced.
- Automatic enqueue uses the advisory turn-end hook, not a transaction with the
  terminal event. Recovery covers queued work; a later turn or manual dream
  rediscovers experience if the process stops before enqueue.

## Rollout

Worker includes dreaming and its child harnesses inherit it. Hosted execution requires the organization-effective Adoption-grade `dreaming` flag; the shared capability filter also applies to explicit agent configuration and commands. Framework presets share the capability data, while embedded hosts own adoption and Memory authorization. Changelog publication is held until explicit user approval.

## Hosting

In-memory Engine state lasts for the process. SQLite hosting persists maintenance
and rediscovers due work through its supervisor. Platform workers share encrypted
PostgreSQL state through internal-only commands. Embedded `/dream` commands kick local maintenance; the server queues work for its worker dispatcher. Durable hosts still need persistent wakeups. In-process review commands use
the same live Memory filesystem routing as remote workers, including directory
creation and artifact reads; workspace snapshots are not authoritative Memory.
Custom/DO hosts must provide a
persistent store, authorized Memory, and durable wakeups; volatile defaults are
not proof of durable support.

Legacy-to-authority cutover refuses dreaming configuration or retained maintenance
state: its import image cannot carry the jobs and Memory-wide reservations without
changing their ownership. This preserves the original authority instead of silently
resetting cursors or spending limits. See the [cutover boundary](../framework/local-authority-cutover.md).

## Evidence

[Runner tests](../../crates/core/src/engine/dreaming/tests.rs) own deterministic recovery
and isolation checks. The [real-model example](../../crates/everruns/examples/dreaming.rs)
and [eval cases](../../crates/everruns/examples/data/dreaming-cases.json) cover
retention and correction behavior. Public usage is in [Dreaming](../../docs/features/dreaming.md).
