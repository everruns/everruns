---
type: Specification
title: "Scalable Engine Boundary"
description: "Portable agent identity, durable orchestration ownership, conformance, and migration rules."
tags:
  - everruns
  - framework
  - engine
  - durable
---
# Scalable Engine Boundary

## Decision

Option 1 is the supported distributed architecture. `everruns-scale` is a
public implementation of `everruns::Engine`; it depends on neither server nor
worker. `everruns-durable` remains the generic workflow substrate. Scale owns
the neutral durable turn driver, portable catalog, canonical PostgreSQL event
log, Environment binding persistence, and blank-database migration ledger.
Server and worker adapt transports and product stores around Scale. They do not
own another Agent runner strategy.

## Definition boundary

`everruns::Agent` is immutable behavior but not a wire value. It may contain
closures, provider drivers, event sinks, workspace providers, hooks, and code
capability implementations. InMemoryEngine accepts that full process-local
graph. Scale rejects every raw Agent before a session or workflow write and
accepts only versioned `PortableAgent` values.

A portable value contains behavior text plus stable registration paths for its
provider, workspace, tools, capabilities, and hooks. Runtime implementations
remain in a bootstrap Registry. A missing or duplicate reference is a typed
error containing the component category and registration path. Driver, sink,
closure, function-pointer, and arbitrary-code fields do not exist in the
portable builder or persistence schema.

## Authority and conformance

The Scale PostgreSQL event log is the single canonical conversation writer. It
assigns identity and sequence under a per-session transaction lock; history is
a bounded projection from those envelopes. Durable workflow events control
scheduling, recovery, steering, and cancellation but do not duplicate
conversation events.

For the same behavior definition, InMemoryEngine and ScaleEngine must agree on
turn outcome, canonical event meaning, history projection, resume behavior,
cancellation, and steering receipts. Differences are limited to durability and
admission: InMemory is process-local and accepts code; Scale survives process
restart and admits references only.

Environment bindings persist independently of Agent construction. Resume must
load the exact WorkspaceHead binding and require its registered provider to
reopen it. Substituting a default head is forbidden. A registered workspace is
deployment-portable only when every eligible worker can access its provider or
trusted shared root.

## Migration and release contract

Embedded applications inventory executable components, assign stable paths,
bootstrap the Registry on every worker, and submit new sessions as portable
definitions. Existing local sessions remain owned by their original engine;
there is no automatic serialization of captured code or local-only workspace
state.

Publish order is durable substrate, Framework/host dependencies, then Scale.
The external consumer builds under `-D warnings`. Architecture guards reject
server/worker dependencies in Scale, product dependencies in durable, a second
AgentRunner/DurableRunner strategy, and any attempt to serialize Agent. Scale
schema migrations use their own ledger so a blank external database and the
larger product database share one idempotent setup path.

## Security constraints

Portable definitions are bounded and contain no credentials. Registration
paths are restricted identifiers rather than URLs. Resolution happens before
persistence. Static parameterized SQL owns event and binding writes; advisory
locks protect event sequences and isolated-head claims. Product ingress retains
organization authorization before invoking the neutral RunController. A public
ScaleEngine instance is an application trust-domain service, not a tenant-aware
HTTP authorization layer.
