---
type: Design
title: "Assignment-scoped Journal Read Foundation"
description: "The first server-side assignment scope, its fail-closed RPC gate, and the dependencies that prevent production activation."
tags:
  - everruns
  - security
  - workers
  - tenancy
---
# Assignment-scoped Journal Read Foundation

Status: **Preparatory server implementation; no production identity adapter or
isolated-worker mode.** The shared worker bearer still has cross-org authority
under [TM-AUTHZ-002](threat-model.md). This change does not protect tenants from
a compromised legacy/shared worker or host.

## Implemented boundary

A trusted server identity adapter can attach an
[actor journal scope](../../crates/server/src/worker_link/grpc_service/assignment_scope/mod.rs)
to request extensions. The value is not a credential, identity verifier or wire
format. Its constructor validates shape only. No production adapter currently
creates it, and request metadata cannot supply or change it. An adapter must
verify executor identity and current assignment authority before constructing
one; the shared bearer is insufficient proof.

The [worker RPC delegation gate](../../crates/server/src/worker_link/grpc_service/worker_service_impl.rs)
denies every operation except actor-journal reads when that scope is present.
New delegated RPCs inherit denial. Generic command dispatch, provider credential
resolution, work claiming, event publishing and notification discovery do not
fall back to internal/Owner authority for such a request.

The [journal handler](../../crates/server/src/worker_link/grpc_service/worker/actors.rs)
rejects a contradictory session selector before reading its data. Each permitted
read performs [online admission](../../crates/server/src/storage/repositories/session_actors/journal_admission.rs)
against the session's org and live actor lease, including its holder, generation
and expiry. Org consistency includes both the session and actor index. No
positive admission is cached across calls or reconnects; database failure does
not grant access. Foreign and absent selectors receive the same refusal.

Admission linearizes at the database statement's snapshot. A read admitted
before lease release/reassignment may finish afterward; already delivered bytes
cannot be retracted. A revocation committed before a new admission is refused.
Capability/grant revocation beyond the actor lease is not implemented here.

## Qualification of this slice

[Network regressions](../../crates/server/src/worker_link/grpc_service/tests/assignment_scope.rs)
use the actual gRPC service and isolated PostgreSQL with two fixture tenants.
A fixed trusted test interceptor stands in for identity integration; it issues
no credentials and is not evidence of production workload authentication.

The counterexample gives one holder leases in both tenants with equal numeric
fences. The scoped reader can read its own journal but cannot choose the other
session, spoof scope through metadata, publish events, fetch provider credentials,
claim work or enter generic command dispatch. Tests also cover release, expiry,
stale generations and reconnect. This verifies the server component's conditional
boundary, not an end-to-end worker-isolation guarantee.

## Dependencies before activation

The broader [assignment-bound design proposal](https://github.com/everruns/everruns/pull/4987)
requires a separate executor identity and capability issuer, online capability
and grant revocation, protected-assignment routing/storage and a security version
floor. Every legacy and alternate route must refuse protected assignments.
Missing request extensions currently retain the trusted legacy protocol;
inserting this scope on one path cannot close those other routes.

All remaining RPCs and decoded commands, HTTP blob paths, direct adapters and
hosting-wrapper HTTP/signing authority must migrate through the same admission
boundary. Executors must be isolated from the scheduler and from other tenants'
authority; broad provider/OAuth credentials require scoped upstream credentials
or trusted relays. Ordered bounded event ingress remains separate work. Until
these dependencies and adversarial deployment qualification pass, the scoped
reader is not a runnable replacement for a worker and must not be presented as
production tenant isolation.

No live identity, credential, certificate, broker user, network/security policy,
or persistent-access setup is performed by this source change. Those deployment
operations require separate per-action approval.
