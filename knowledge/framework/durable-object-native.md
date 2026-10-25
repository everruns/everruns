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

**Status: implemented, source-only experimental reference.** Native and Wasm placements enter
`everruns::Engine` and execute ordinary Framework Agents through the same shared
Rust session driver. Placement changes the authority transport and explicit host
services, rather than the agent runtime. The
[Engine-owned sessions decision](engine-durable-sessions.md) owns that boundary.

The adapter resolves in its own Cargo workspace using only the Framework
hosting SPI. Neither it nor the celld serve adapter belongs to the product
publish set. The Engine architecture supports external authorities; owning a
Cloudflare service integration is not a Framework responsibility.

## Decision

One authority owns a session's admitted input, canonical journal, execution epoch,
inbox progress and next activation. A native executor or a Wasm isolate reconstructs an
ordinary Framework agent or registered serve agent and runs the shared Rust
session actor driver. JavaScript owns storage and activation. The object invokes the shared Rust
session-command implementation, compiled to Wasm, within its storage transaction.
This applies to both native and isolate Engine placements: canonical pending calls,
deadlines and receipts are read where the decision commits. Remote commands cannot
supply prepared permission writes. Agent planning and session-command semantics
are never reimplemented in JavaScript.
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

The public extension is `SessionHosting` beneath Engine. It owns binding,
transactional admission, commands, epoch-scoped runtime ports and independent
activation. Framework start/completion hooks, durable outcomes, context inspection
and admission handles stay in the Framework. serve registers ordinary Agents and
uses its same Engine for native `/v1` and authenticated `/execute` requests.
Applications never construct a core runtime or use an authority client to send
user input directly.

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
needed. Framework lifecycle and approval waits use durable receipts. Background processes,
channels and schedules need independent host-service contracts; unsupported
requirements fail before listening or admission. Legacy local/custom-backend
facade execution retains its separately documented weaker boundary.

## Hibernation and passive observation

The isolate Engine releases ownership during persisted provider backoff. Rust
retains the deadline and retry budget; the existing object alarm supplies the next
activation. No JavaScript planner or heap snapshot is involved. Active network
requests and arbitrary user tool timers remain active work, not hibernated code.

The Workers SSE bridge subscribes to committed-event hints over a hibernating
WebSocket. Its heartbeat runs in the frontend Worker, so an idle stream no longer
polls the session DO. Notifications carry no event payload and are not a source of
truth: the observer replays bounded canonical pages with its existing cursor.
Celld 0.6.2 needs a complete HTTP upgrade through a fixed notification ingress:
its kept cross-node stub response can connect without delivering later frames.
The origin is deployment-controlled, never taken from the caller's URL, and
redirects are refused before any authority credential can be forwarded elsewhere.
The fleet qualification idles observers on both surviving frontends, then verifies
new input delivery after the client-free recovery proof has already completed.
Connection loss closes SSE so clients reconnect and replay. The internal watch
endpoint requires the authority credential and matching session identity, accepts
no commands and bounds connection count. Other clients that actively poll still
wake the object.

The [workerd hibernation proof](../../crates/durable-object/worker/test/hibernation.mjs)
checks that an ordinary pending timer blocks eviction, then evicts a parked Engine
with SSE connected and requires the alarm to reopen it and finish the original
admission without client traffic. This is separate from forced-reset recovery.
It qualifies the pinned local workerd runtime; it does not assert a production
Cloudflare eviction schedule or a Vercel hosting integration.

## Identity and trust

The authority and executor use separate service credentials. Tenant-qualified
session addressing and immutable agent/build bindings prevent accidental cross-
tenant or wrong-build activation. Embedders authenticate users and authorize
session access before calling the privileged authority API. Service credentials
must never reach browsers or agent tools. An executor URL is deployment config,
not input supplied by a request; redirects are refused. Agent build identifiers name immutable behavior/configuration retained for the
session's retention period. They are independent of deployment digests and compatible
Engine upgrades. Engine checks the persisted runtime format separately. See the
[retained-registration strategy](engine-durable-sessions.md#agent-registration-and-creation)
for compatibility diagnostics and safe retry after restoring missing code.

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
ordinary Framework Agent/Engine, capability/tool APIs, shared actor driver and
canonical turn loop inside the object. The [isolate qualification test](../../crates/durable-object/worker/test/qualify-isolate.mjs)
loses the isolate after a committed session KV effect, then permits no application
requests during recovery. Its first subsequent event read must already contain a
completed turn. It also checks immutable build binding, admission deduplication,
stale-write refusal and interrupted at-most-once tool recovery.

Backend qualification is specific to the platform and failure tested. Local
workerd evidence is not evidence of Cloudflare production service behavior, and a
local replicated celld fleet is not evidence of a particular cloud bucket's
consistency, network topology or operations. Keep those limits with the published
validation results. The Engine migration replaces the older snapshot/replay example; its canonical
snapshots are not a supported authority format.

The Cloudflare service qualification passed isolate abort after the committed
counter effect, no client requests during recovery, one interrupted result and
stale epoch refusal. Temporary Worker and namespace cleanup was verified through
the account API. The three-node celld qualification passed owner process SIGKILL
with the same assertions against its replicated fleet and a local serialized S3
filesystem emulator. Its orphan-alarm scan is configured for the test's recovery
budget; production scan cadence affects recovery latency. These proofs do not
establish cloud bucket or network-partition guarantees beyond the tested setup.

Native serve qualification also runs the Engine executor in an actual Docker
container. One test kills that container; another keeps it alive while killing
the authority owner in a three-node celld fleet. Both observe through ordinary
`/v1` only after a period with no application requests, verify one admission and
canonical dispatch, and create another session while the first is busy. The
[test harness](../../examples/serve/celld/worker/test/e2e.mjs) uses a pinned runtime
image and synthetic provider; it qualifies recovery, not an untrusted OS-tool
sandbox. Native OS tooling requires an explicit sandbox adapter isolating agent
commands from Engine service credentials and other sessions.
