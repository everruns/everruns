---
type: Decision
title: Shared Execution with Durable Object Authority
description: Native and Wasm execution over a Durable Object journal using the same Rust session recovery and tool loop.
tags:
  - everruns
  - framework
  - durable
  - experimental
---

# Shared Execution with Durable Object Authority

**Status: implemented, experimental, source-only.** Native and Wasm placements
reuse the same Rust host and session driver from
[Portable Durable Agent Execution](durable-execution-portability.md). Placement
changes the injected transport and services, not agent execution semantics.

The [Engine-owned sessions proposal](engine-durable-sessions.md) revises the
Framework integration target. The current adapter shares the runtime but enters
through a core host factory; the Wasm example does not construct a public
`everruns::Engine`. Framework sends and serve's ordinary routes therefore do not
yet gain DO authority from this adapter. The proposed integration makes Engine
own the full session contract, including durable admission and lifecycle hooks.

## Decision

One authority owns a session's admitted input, canonical journal, execution epoch,
inbox progress and next activation. A native executor or a Wasm isolate reconstructs an
ordinary Framework agent or registered serve agent and runs the shared Rust
session actor driver. JavaScript owns storage and activation, never agent planning.
Native placement keeps native tools and providers available. Isolate placement
uses the shared provider protocols over host fetch and only tools supported by
its injected services. Protocol encoding, SSE parsing, retry classification and
usage remain shared Rust code; the object bridge moves HTTP bytes.

The [core driver](../../crates/core/src/host/session_actor/mod.rs) is shared by
native platform workers and the [object executor](../../crates/durable-object/src/executor.rs).
Fleet discovery remains a platform concern; an object claims a known session.
The optional portable execution host composes the same implementation files;
default core features remain empty. Fleet discovery extends the single-session
ownership port, so object adapters never implement unsupported fleet methods.

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

The [Wasm application](../../examples/durable-object-agent/README.md) runs the
ordinary host builder, capability/tool SPI, shared actor driver and canonical
turn loop inside the object. The [isolate qualification test](../../crates/durable-object/worker/test/qualify-isolate.mjs)
loses the isolate after a committed session KV effect, then permits no application
requests during recovery. Its first subsequent event read must already contain a
completed turn. It also checks immutable build binding, admission deduplication,
stale-write refusal and interrupted at-most-once tool recovery.

Backend qualification is specific to the platform and failure tested. Local
workerd evidence is not evidence of Cloudflare production service behavior, and a
local replicated celld fleet is not evidence of a particular cloud bucket's
consistency, network topology or operations. Keep those limits with the published
validation results. The older snapshot/replay example retains its documented
weaker tool replay semantics.

The Cloudflare service qualification passed isolate abort after the committed
counter effect, no client requests during recovery, one interrupted result and
stale epoch refusal. Temporary Worker and namespace cleanup was verified through
the account API. The three-node celld qualification passed owner process SIGKILL
with the same assertions against its replicated fleet and a local serialized S3
filesystem emulator. Its orphan-alarm scan is configured for the test's recovery
budget; production scan cadence affects recovery latency. These proofs do not
establish cloud bucket or network-partition guarantees beyond the tested setup.
