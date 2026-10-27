---
type: Playbook
title: "Worker Isolation Qualification and Rollout"
description: "Proposed stages, compatibility rules and adversarial evidence required before claiming assignment-bound tenant isolation."
tags:
  - everruns
  - security
  - workers
  - testing
  - deployment
---
# Worker Isolation Qualification and Rollout

Status: **Proposed gates; not executed.** This accompanies the
[assignment-bound design](worker-assignment-isolation.md). Current shared workers
still rely on [TM-AUTHZ-002](threat-model.md). Passing an event-stream test or
merging a design is not evidence that the stronger boundary exists.

## Stages and exit evidence

### 0. Freeze the inventory and threat boundary

Enumerate the compiled worker service descriptors, decoded generic operations,
command registry, internal HTTP byte routes, direct adapters, and background
dispatch/reaper paths. Each operation declares its service role, assignment
purpose, target scope, side-effect class and revocation/replay requirements.
An unclassified operation is denied and fails coverage validation. The present
65-RPC/19-handler-group source baseline is only the starting census.

Identify every credential, database handle and provider/sandbox control channel
an executor can reach through memory, environment, filesystem, IPC and network.
Classify scheduler, provider relay and cleanup services explicitly as trusted
or give them narrower resource assignments; do not hide another shared worker
behind a different service name.

Exit: source inventory has no unmapped operation; threat review states the
attacker positions, trusted services and host/side-channel exclusions. No new
isolation claim is made at this stage.

### 1. Add the server authorization and protocol foundation

Implement executor/scheduler identity separation, the server-owned assignment
record, online admission, scoped runtime caller and capability negotiation.
Keep capability issuance inaccessible to executor/legacy credentials. Specify
lease/grant revocation races, replay receipts and server failover before adding
fast paths. New protocol fields and storage schemas belong in their source
owners, not in a second specification copy.

Additive server support may coexist with the old protocol only for trusted
legacy workloads. It must not automatically enable enforcement, create live
credentials or change live deployment policy through a source merge.

Exit: local two-tenant fixtures exercise the authorizer through the real network
adapters; forged scope is refused before storage, broker or relay access. Live
identity/credential setup remains a separate approved action.

### 2. Migrate every authority family

Move reads, writes, credential/resource resolution, actor/task operations,
generic command dispatch, byte upload/download, sandbox persistence and external
side effects onto the same authorization edge. Leave global claim/reaper and
notification discovery with the scheduler. A scoped executor cannot fall back
to `Caller::internal`, raw deployment secrets or a direct database adapter.

Preserve intentionally authorized management and delegation through explicit
server-side invocation purposes and current resource grants. A feature with no
scoped replacement is an explicit migration blocker; do not silently disable
it or widen the capability to regain compatibility.

Exit: descriptor/registry coverage is complete; each family has positive and
negative network tests; all legacy alternate paths reject protected assignments.
Credential-relay and accounting behavior are qualified without real paid calls.

### 3. Qualify the isolated executor and stream

Run a deliberately malicious executor process with a real valid tenant-A
assignment. It must have no scheduler bootstrap authority, other tenants'
capabilities, deployment encryption/provider keys, database/broker credentials,
or readable shared control sockets/volumes. Qualify sandbox credential exclusion
separately. A process/container check alone cannot attest host-compromise safety;
a guest-VM escape test does not attest resistance to a compromised hypervisor.

Exercise the real gRPC and HTTP listeners, server, local PostgreSQL, NATS and SSE
with two tenants and simulated model/provider/payment endpoints. Test bounded
queues, revocation, replay and recovery under injected latency/faults. Measure
the full enabled authorization path, not a standalone serializer or a version
with checks disabled. Keep raw commands/configuration and results with the run
artifact; do not put tokens, prompts or provider secrets in the report.

Exit: the adversarial matrix below passes for the exact revision; measured
resource and latency limits are recorded; no unexplained failures or missing
coverage remain. This is local qualification, not a production attestation.

### 4. Approve and attest deployment preparation

Prepare a concrete manifest/identity/credential diff, binary/version matrix,
workload-to-role inventory, expected cost/capacity and rollback plan. Obtain
separate per-action approval before changing live network/security policy,
IAM/RBAC, credentials, certificates, broker users or secret delivery. Name the
exact target and consequences for each action; approval of this design or a
generic source merge is not approval for those mutations.

Keep the server as the only broker participant. Do not add an executor NATS
user as a substitute for application authorization. Separate scheduler identity
and executor credential delivery must be attested from the actual deployed
configuration and process boundary, not inferred from source defaults.

A canary may validate compatibility/performance while legacy workers remain
trusted, but cannot establish protection from compromise of a legacy shared
worker that retains a route to the protected tenant. Every old server replica,
global claim path, shared credential and alternate byte/command path must be
accounted for before making that claim.

### 5. Cut over with a security version floor

Drain and account for in-flight legacy work. Ensure all server replicas enforce
the required protocol/policy version before routing protected assignments.
Retire legacy executor authority and remove shared secret delivery through
individually approved operations. Previously exposed credential families need
an explicit revocation/rotation decision; do not assume process termination
revokes a copied secret or automatically rotate unrelated broker credentials.

Repeat adversarial tests against approved deployed fixtures without accessing
unrelated tenant data or creating paid model traffic. Record exact source,
images, identities, policy versions and relevant resource/route admission state.
Only this evidence, together with the qualified threat boundary, can support a
bounded isolation claim. Unverified hosts or residual legacy routes stay named
limitations.

## Compatibility and rollback

| Combination | Required behavior |
|---|---|
| Legacy executor and legacy server | Existing trusted-worker model; no stronger guarantee |
| Legacy executor and upgraded server | Trusted legacy pool only; cannot claim protected assignments, mint scoped authority or address protected resources through any route |
| Scoped executor and legacy/unknown server | Fail closed; no shared-token, environment-variable or internal-caller fallback |
| Scoped executor and upgraded server | Negotiate identity-bound assignment authority, actual delivery capabilities and bounded limits before work |
| Mixed old/new server replicas | Not qualified for strict isolation until the enforcing version floor is attested everywhere |

Strict assignments cannot be returned to legacy workers by an automatic fallback
or ordinary binary rollback. Roll back only to a qualified enforcing version,
or pause/drain the protected workload. Re-enabling broad legacy authority is a
security change requiring its own approval, not an availability repair shortcut.
Data migrations must be additive until the strict-compatible rollback window
closes; old code must not be allowed to ignore an assignment-security marker.

## Adversarial qualification matrix

For every row, verify the result at the real adapter and the resulting state:
tenant-B data/bytes remain undisclosed, no unauthorized database/broker/relay
effect occurs, and errors do not reveal whether a foreign resource exists.
Record positive own-assignment behavior alongside rejection evidence.

| Attack/fault | Required evidence |
|---|---|
| Valid A capability with B tenant, session, message/cursor, invocation, actor, resource, sandbox or parent IDs | Reject inconsistent scope before any protected access; test absent, guessed, valid and malformed selectors through both cold reads and warm caches |
| Missing/invalid capability, API token substituted for worker authority, executor spoofing scheduler/another executor | Refuse authentication/role; no registration, global claim, cleanup scan or queue discovery |
| Valid outer generic RPC with forbidden inner command, user/principal, platform or JSON operation | Decoded operation passes the same scoped authorizer; unknown new descriptors/verbs fail coverage and runtime admission |
| Forged event type, protected provenance, human-input marker or accounting metadata | Runtime events cannot mint management/payment/grant authority; deferred listeners retain the server-derived assignment context and trusted relay receipts |
| Actor/task reassignment, cancellation, capability expiry/renewal, grant revocation during an open stream | Old generation/attempt cannot make a new admission; trace the linearization race and distinguish pre-revocation admitted work |
| Read/grant revocation during large byte transfer | New bounded chunks stop; retained handles, signed URLs and image-upload routes cannot bypass the check; already-delivered bytes are not claimed retractable |
| Exact retry after lost receipt; same ordinal with changed body; gap/out-of-order/cross-assignment replay | Return a prior valid receipt or refuse replay without a second mutation/charge; never reset replay state on reconnect |
| Server restart/failover and stale admission cache or capability | New authority fences the old generation; unavailable authoritative state fails closed rather than allowing from a stale cache |
| Blob signature/query tampering, old shared signer, foreign upload metadata, alternate direct adapter | No byte read/write or internal-caller/database escape; executor has no signer/root key/storage credential |
| Provider/MCP/Slack/payment target or initiator spoofing, shared deployment-key request | Current resource grants, management purpose, egress/spend policy and side-effect idempotency apply; no raw shared secret is returned |
| Executor/sandbox environment, filesystem, process and IPC inspection | Only permitted assignment authority is present; scheduler/other tenant secrets are absent; actual host boundary limitations recorded |
| Oversize/malformed batches, slow consumer, reconnect storm, concurrent streams, one noisy tenant | Count/byte/in-flight budgets hold; fair progress and bounded memory are measured; completion cannot overtake accepted prior work |
| Broker restart/missing stream, changed subject policy, lost/uncertain publish acknowledgement | Restore only a definite missing buffer, without policy rewrite or blind uncertain retry; verify actual SSE delta/final delivery and durable transcript recovery |
| Connection drops at each admission/commit/publish/ack boundary | Durable results remain idempotent and recoverable; ephemeral loss is surfaced honestly and does not become persisted token bodies or a false delivery receipt |
| Old server/executor or rollback configuration reintroduced | Protected assignments remain inaccessible; admission refuses an unsupported security version instead of downgrading |

The performance report must include first-delta, inter-delta and terminal latency,
throughput, loss/coalescing counts, database operations, memory ceilings and
cross-tenant fairness with enforcement enabled. Use simulated providers in tests;
paid production probes are not a prerequisite to local qualification.

## Claim and completion gate

The final report names the exact attacker position and trusted components,
source-versus-live state, all tested adapters, runtime identity/policy attestation,
unresolved exclusions and rollback floor. No worker-compromise guarantee is made
while any executor can obtain shared cross-tenant authority. No host-compromise
guarantee is made without a separate host-boundary qualification.
