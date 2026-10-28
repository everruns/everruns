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

The [canonical worker authority](../../crates/server/src/worker_link/grpc_service/authority/mod.rs)
is a server-owned request context shared by RPC and HTTP. The existing bearer
authenticator explicitly establishes trusted legacy authority, or development
authority when authentication is deliberately disabled. Missing context is
unauthenticated; it never implies legacy access. Reauthentication refuses an
already-established context rather than overwriting it.

A trusted server identity adapter can establish journal-only authority with an
[actor journal scope](../../crates/server/src/worker_link/grpc_service/assignment_scope/mod.rs).
The value is not a credential, identity verifier or wire format. Its constructor validates shape only. No production adapter currently
creates it, and request metadata cannot supply or change it. An adapter must
verify executor identity and current assignment authority before constructing
one; the shared bearer is insufficient proof.

The [worker RPC delegation gate](../../crates/server/src/worker_link/grpc_service/worker_service_impl.rs)
requires explicit authority for every delegated operation and denies every
operation except actor-journal reads for journal-only authority.
New delegated RPCs inherit denial. Generic command dispatch, provider credential
resolution, work claiming, event publishing and notification discovery do not
fall back to internal/Owner authority for such a request.

The same listener's [blob HTTP routes](../../crates/server/src/api/internal_blobs.rs)
refuse journal-only scopes before path/query extraction, upload-body buffering,
signature processing or storage. Even an otherwise-valid legacy signed URL does
not grant byte access to this role. Missing or ambiguous authority is refused
before any body or storage access. Explicit trusted legacy/development authority
retains its existing bearer/signature behavior. A standalone scope from an old
adapter is rejected even beside canonical legacy authority; silently ignoring
that marker would downgrade a scoped request to broad access.

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
The journal authorization grant is independently revocable. The unactivated
[verified journal path](../operations/durable-execution-engine.md#worker-communication)
now checks exact mTLS executor enrollment, opaque proof and explicit supporting
session-service grants to organization-owned service resources. Lifecycle triggers revoke linked assignments;
complete user/custom-policy provenance, issuance and production activation
remain unimplemented.

## Independent journal authorization

A [stored journal-read grant](../../crates/server/migrations/20261028074331_worker_journal_read_grants.sql)
is now mandatory for scoped reads. Admission checks its executor and assignment
binding, database-clock validity and revocation together with the live lease
in one statement. An active lease alone cannot preserve a revoked or expired
authorization. A new grant for the same executor and lease does not resurrect
a retained old reference. Missing, foreign and mismatched grants all refuse
without revealing a foreign journal.

These records and executor references are not credentials. Matching an executor
identifier does not authenticate a workload. The unactivated verified adapter
resolves it from exact mTLS peer enrollment and a stored opaque capability;
caller headers and grant IDs are insufficient. No production
producer creates grants or scopes, and no worker/public API exposes grant
creation or the trusted control plane's org-scoped revocation operation.

Grant bindings and deadlines are immutable, revocation is irreversible, and
records remain as tombstones while their parent session and org are live.
Ordinary row deletion cannot recycle a revoked reference. Parent deletion
removes the grant along with its data. Database owner/control-plane compromise
remains outside this component's trust boundary. Deadlines are finite and
bounded by the actor maximum lease window; effective admission ends at the
earlier grant or actual lease expiry. Renewing a lease never extends the grant.

[Grant regressions](../../crates/server/src/worker_link/grpc_service/tests/assignment_scope/grants.rs)
first reproduced six lease-only admission failures, including foreign journal
reads with a contradictory grant binding and equal holder/fence values. They
exercise revocation without releasing the lease, reconnect/replay after
replacement, unknown grants, expired/not-yet-valid grants and cross-executor
or cross-tenant substitution. Separate database tests enforce immutable
bindings/deadlines, tombstone retention and tenant-scoped revocation. The
executor identity adapter in these tests is a synthetic server assertion, not
production identity or credential qualification.

## Qualification of this slice

[Network regressions](../../crates/server/src/worker_link/grpc_service/tests/assignment_scope/mod.rs)
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
requires production executor enrollment and capability issuance, complete
underlying user/custom-policy revocation provenance, protected-assignment
routing/storage and a security version floor. The source verifier and declared
session-service dependency revocation cover only part of that boundary. Every legacy and alternate route must refuse protected assignments.
Standard bearer-authenticated wire clients keep their existing protocol. Custom
server adapters and direct calls to the public RPC implementation must establish
explicit server-owned authority; bypassing the authenticator now fails closed.
This internal adapter requirement does not bind legacy workers to assignments,
and inserting scoped authority on one path cannot close other legacy routes.

All remaining RPCs and decoded commands, HTTP blob paths, direct adapters and
hosting-wrapper HTTP/signing authority must migrate through the same admission
boundary. Executors must be isolated from the scheduler and from other tenants'
authority; broad provider/OAuth credentials require scoped upstream credentials
or trusted relays. Ordered bounded event ingress remains separate work. Until
these dependencies and adversarial deployment qualification pass, the scoped
reader is not a runnable replacement for a worker and must not be presented as
production tenant isolation.

[Combined-listener regressions](../../crates/server/src/worker_link/grpc_service/tests/scoped_blobs/mod.rs)
exercise the actual outer bearer layer and merged gRPC/HTTP routes with a fixed
trusted scope adapter. They cover signed image/file URLs, own/foreign uploads,
malformed requests, stalled/oversized bodies and unavailable storage, alongside
legacy positive and authentication/signature negative cases. Fault-injection
adapters validate a bearer and then lose its context or attach an obsolete scope
marker; both transports refuse these requests even with forged role headers or
a valid signed URL for another tenant. An instrumented
router body must receive zero polls on refusal; network refusal must not wait
for a stalled upload to finish. These are synthetic component tests, not
production identity or deployment qualification. Additional regressions refuse
a public RPC invoked without an auth adapter and reject a context-free client
event stream without waiting for end-of-stream.

The canonical context adds in-memory checks only; online journal admission is
unchanged. Standard server bootstrap establishes authority in its existing
outer interceptor on both transports. Deploying a custom adapter requires
updating that trusted adapter first; a bare service or lost context now returns
unauthenticated instead of inheriting broad access. This does not establish a
security version floor across replicas, authenticate a workload, or revoke any
legacy bearer. Those are prerequisites before production scope issuance.

No live identity, credential, certificate, broker user, network/security policy,
or persistent-access setup is performed by this source change. Those deployment
operations require separate per-action approval.
