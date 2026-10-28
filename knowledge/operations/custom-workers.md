---
type: Decision
title: Customer-hosted custom agent workers
description: Custom agent implementations connect through a restricted, agent-scoped worker protocol while the control plane owns session intake and canonical events.
tags:
  - everruns
  - workers
  - security
  - experimental
---

# Customer-hosted custom agent workers

Status: experimental, self-hosted and operator-provisioned. Cloud deployment of
customer images is outside this change.

## Intent and ownership

An agent author can package custom execution logic, Framework tools and private
data access in their own process or container. Clients continue to use the
platform's ordinary session APIs, chat UI and SSE. The control plane owns input,
routing, session leases, event storage and observability fan-out. The customer's
code owns model access, tool execution and its external resources.

The existing trusted worker link cannot be exposed for this purpose. Its shared
credential grants internal cross-org access. The custom service is a separate
listener with its own agent-scoped credential lookup; it never forwards arbitrary
RPCs or domain commands to the trusted service. The listener is absent unless
explicitly configured and requires TLS outside loopback. See
[the protocol](../../crates/internal-protocol/proto/worker.proto) and
[the service](../../crates/server/src/worker_link/custom_workers/mod.rs).

## Routing and authority

A deployment binds one agent in one organization. Operator provisioning is
intentionally separate from ordinary tenant-controlled agent configuration. Only
a SHA-256 digest of a high-entropy worker token is stored. Each request checks the
current enabled deployment; disabling it revokes subsequent requests immediately.
An in-flight request authorized before revocation may finish.

The binding is persistent, including when its credential is disabled. Built-in
workers exclude bound sessions before their bounded candidate scan; a custom
worker leases only its own bound agent. An unavailable custom implementation
therefore leaves input queued instead of silently executing different code. Both
the remote and in-process built-in workers use the same repository rule. Stop
workers and finish or cancel active turns before binding, rebinding or deleting a
deployment; deleting the binding explicitly returns its sessions to built-in
execution. Provisioning is not a live migration of a held lease.

Customer-selected replica identities are namespaced by the deployment id on the
server. Every journal read, renewal, release and event submission verifies the
owning organization, bound agent, holder, fence and live lease. There is no
customer-supplied organization selector. Durable event writes reuse the existing
transactional append fence. Input identity must be a control-plane-recorded
invocation of that session. Event types are explicitly limited to execution;
intake, approvals, configuration and platform commands remain unavailable.
The CP derives turn and trace correlation ids from the deployment, session and
recorded input; customer-supplied span identities are namespaced so a customer
cannot close or parent to another tenant's spans in a global observer map.
Every event carries server-stamped deployment provenance; customer generation
reports cannot claim a control-plane provider account for managed-spend attribution.
Terminal and start events update the session status and synthesize platform
lifecycle events, so ordinary UI and trace listeners observe them.

The protocol deliberately exposes neither provider credentials nor private
connection resolvers, encryption keys, management identities, global queues,
filesystem tools or arbitrary storage commands. The customer's database and model
credentials stay in the worker's own deployment.

## Execution and recovery

The existing actor lease remains the scheduler and the canonical session log is
the recovery input. A worker renews its lease while it executes, stops on renewal
failure, handles cancellation, and releases only the input it actually completed.
New input remains runnable. The control plane accepts only typed, scoped events;
it sends them through the ordinary EventService for persistence, SSE, listeners,
usage projections and tracing. Streaming deltas retain the existing ephemeral
delivery semantics; durable append fencing does not make deltas durable.

A custom implementation is responsible for truthful event reporting, recovery,
tool approvals, budgets and policy inside its own process. The CP does not claim
that arbitrary code obeys the built-in Act pipeline. Remote side effects need
their own idempotency and interruption policy; a lease does not undo a database
write. A new runtime must qualify these behaviors before claiming native-runtime
recovery guarantees.

[The Framework data-agent example](../../examples/custom-data-agent/README.md)
uses a private read-only SQLite database and a real model. It forwards canonical
Framework events while retaining CP session/input/turn identity, handles cancels,
and replays interrupted reads with a bounded attempt count. It reconstructs prior
conversation for each turn rather than claiming to persist a second Framework
Engine. A final answer that committed before a crash can finish without querying
again. Streaming observer loss is an error, never a successful acknowledgment.
The example intentionally does not implement approvals, client tools, delegation,
background tasks, durable arbitrary side effects or hosted secret access.

## Isolation boundary and qualification

This feature authenticates a remote executor; it does not sandbox it. Operators
run customer containers outside the control-plane trust domain with no CP database
credentials, host mounts, internal fleet token or privileged network access.
Arbitrary custom images remain disabled in Cloud until a separately reviewed
compute, filesystem, secret and network isolation boundary exists.

Regression tests cover persistent routing without fallback, revocation, credential
rejection, cross-deployment/tenant and session denial, live lease and fence checks,
and canonical event ingestion. Source:
[custom worker tests](../../crates/server/src/worker_link/custom_workers/tests.rs).
The sample's database test verifies aggregate results and parameter binding.
The example's fixed-origin OpenAI transport preserves deployment proxies and CA
trust while rejecting other origins and redirects; it does not relax native
platform HTTP guards. The complete real-model smoke runner owns disposable
fixtures, tests process/container replacement and credential rotation, checks the
ordinary API/SSE/work surfaces and records machine-readable private evidence.
Python harness regressions run through the example's cargo test target. See the
[agent acceptance case](../test-cases/agents/custom_workers/TC001_custom_data_worker_smoke.md)
and [UI acceptance case](../test-cases/ui/custom_workers/TC001_custom_worker_chat_and_trace.md)
for qualification; run records are kept outside the knowledge bundle.
