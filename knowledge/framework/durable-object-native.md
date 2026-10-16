---
type: Decision
title: Native Execution with Durable Object Authority
description: Experimental native Rust execution over a Durable Object journal, with one shared recovery driver and explicit backend qualification limits.
tags:
  - everruns
  - framework
  - durable
  - experimental
---

# Native Execution with Durable Object Authority

**Status: implemented, experimental, source-only.** This is the first placement
from [Portable Durable Agent Execution](durable-execution-portability.md).
It reuses the native Rust runtime and moves session authority to a Durable Object.
It is not a qualification of production Cloudflare deployments or celld clusters.
The [guarantees proposal](durable-execution-guarantees.md) remains the target for
additional profiles and qualification.

## Decision

One authority owns a session's admitted input, canonical journal, execution epoch,
inbox progress and next activation. A separate native executor reconstructs an
ordinary Framework agent or registered serve agent and runs the shared Rust
session actor driver. JavaScript owns storage and activation, never agent planning.
This keeps native tools and providers available without making a second runtime.

The [core driver](../../crates/core/src/host/session_actor/mod.rs) is shared by
native platform workers and the [object executor](../../crates/durable-object/src/executor.rs).
Fleet discovery remains a platform concern; an object claims a known session.
The portable default core feature set remains independent of this native driver.

The Framework's experimental backend factory makes turn placement injectable.
serve can accept that Engine or reconstruct registered agents for external hosts.
These are extension seams, not a built-in durable replacement for Framework
`Session::send`: its acknowledgement still has the Engine's existing semantics.
The object profile admits input directly through its authority client.

## Durability boundary

Admission commits the input, its command receipt and an alarm together. Retrying
an accepted message with the same identity returns the receipt; conflicting reuse
is rejected. A successful native callback is not the durable work record. Alarms
continue checking runnable work and lease expiry after the executor disappears.
Transient activation failures persist a retry time and a replacement alarm before
returning, rather than relying solely on the platform's finite failed-alarm retries.
Storage/platform outages beyond the alarm service's guarantees still require
operational recovery; this profile does not claim an independent reconciler.

A claim creates an immutable execution epoch. Every canonical executor mutation
checks both epoch and unexpired authority-clock lease in the same transaction.
A stale process cannot append, release, update checkpoints or change session KV.
Renewal timeout stops the Rust driver; even a process that cannot stop promptly
has no canonical write authority. External effects still require tool-specific
idempotency or reconciliation. Started at-most-once tools recover as interrupted,
without repeating the effect; no exactly-once external-effect claim is made.

Pending client-tool results and their partial resolutions survive process loss.
Cancellation and input cursors preserve newer work across release races. Event
reads are bounded and use a stable watermark for pagination. Credentials and
workspaces are not made durable by a durable journal: the default object profile
refuses secret storage and requires an explicit external workspace/store where
needed. Background processes, schedules, outer application lifecycle hooks and
application approval handlers need their own durable hosting contracts.

## Identity and trust

The authority and executor use separate service credentials. Tenant-qualified
session addressing and immutable agent/build bindings prevent accidental cross-
tenant or wrong-build activation. Embedders authenticate users and authorize
session access before calling the privileged authority API. Service credentials
must never reach browsers or agent tools. An executor URL is deployment config,
not input supplied by a request; redirects are refused. Build identifiers must
name immutable code that remains available for the session's retention period.

See [TM-DURABLE-012](../security/threat-model.md) for the security boundary and
[source configuration](../../crates/durable-object/worker/wrangler.jsonc).

## Evidence and remaining qualification

The [authority tests](../../crates/durable-object/worker/test/authority.test.mjs)
exercise transactional admission/alarm rollback, deduplication, stale ownership,
release races, cancellation, durable partial tool results and bounded snapshots.
[Activation tests](../../crates/durable-object/worker/test/activation.test.mjs)
cover persistent retries, duplicate wakeups, build suspension and stale failures.
The [workerd recovery test](../../crates/durable-object/tests/recovery.rs) kills a
separate native process after its tool effect, starts a replacement, sends no
replacement input and requires alarm-driven completion with one external effect.
It runs in the [recovery workflow](../../.github/workflows/durable-object.yml).

Wasm placement, actual Cloudflare deployment tests, multi-node celld failover,
public session API integration, retention/compaction policy and a full backend
conformance matrix remain separate qualification work. The snapshot/replay celld
example remains an alternative with its previously documented weaker tool replay
semantics. No existing backend or example silently changes its guarantees.
