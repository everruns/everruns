---
type: Specification
title: "Domain Modules"
description: "Domain modules: Command trait, feature-oriented structure, MCP catalog generation."
tags:
  - everruns
  - foundations
---
# Domain Modules

## Purpose

Domain modules own user-facing operations, validation, authorization policy,
business rules, and shared persistence helpers for one feature. HTTP, MCP,
gRPC, and platform callers converge on the same command path so behavior and
policy cannot drift by protocol.

Before this pattern, one operation was often split across an HTTP handler,
service, MCP catalog entry, and bespoke dispatcher. Domain commands make the
operation the unit of reuse and registration.

## Sources of truth

- [`crates/server/src/domains/common.rs`](../../crates/server/src/domains/common.rs)
  owns command traits, metadata, context, errors, policy enforcement,
  instrumentation, schema helpers, and generic dispatch.
- [`crates/server/src/domains/`](../../crates/server/src/domains) contains the
  current domain inventory and concrete command patterns.
- [`crates/server/src/api/dispatch.rs`](../../crates/server/src/api/dispatch.rs)
  owns the HTTP adapter chokepoint.
- [`crates/server/src/api/mcp_endpoint/catalog.rs`](../../crates/server/src/api/mcp_endpoint/catalog.rs)
  owns scripted-tool schema adaptation and safe dispatch error formatting.
- [`crates/server/src/services/platform_command_surface.rs`](../../crates/server/src/services/platform_command_surface.rs)
  owns transport-neutral catalog discovery and bounded `query`/`execute`
  behavior shared by MCP and the built-in `platform` capability.
- [`crates/internal-protocol/proto/`](../../crates/internal-protocol/proto) owns
  exact gRPC messages.
- [`crates/server/tests/command_policy_enforcement_test.rs`](../../crates/server/tests/command_policy_enforcement_test.rs)
  verifies policy coverage from command inventory.

Do not copy the command trait, context fields, error enum, registry descriptor,
or current domain list into this spec. Their definitions and exhaustive tests
are more reliable than a second inventory.

## Module ownership

A user-facing domain normally separates:

- commands: public operations, validation, policy, and orchestration;
- queries: reusable persistence helpers without caller policy;
- types: request, response, and domain-owned storage shapes;
- optional service or runner modules for substantial internal orchestration
  shared by several commands.

This is a responsibility boundary, not a mandatory file template. A small
domain may use fewer files. Internal-only orchestration can live under the
owning domain without manufacturing a public command.

Code belongs in a global `services` module only when no single domain naturally
owns it and it is genuinely cross-cutting infrastructure, registry behavior, or
an external integration boundary. A command reaching into an unrelated global
service for feature logic is a design smell.

## Command contract

A command defines:

- serializable input and output;
- stable catalog metadata;
- input and output schema discovery;
- whether it is read-only;
- optional positional shorthand for scripted MCP use;
- the permission policy;
- execution against shared caller context.

The exact methods and defaults live on `Command` and `CommandSchema` in
`domains/common.rs`.

### One enforcement point

`Command::run` is the public entry point. It evaluates policy using the active
permission resolver, adds protocol-independent tracing and metrics, and then
executes the operation.

HTTP adapters, generated MCP dispatch, gRPC command transport, and platform
capabilities must call `run` or a helper that calls it. They must not call
`execute` directly. Direct execution is reserved for composition inside an
already-authorized command, where re-running policy would be redundant and the
caller remains within the same operation boundary.

Every non-read-only mutation declares policy. Inventory-based tests enforce
coverage. A new caller that appears to require bypassing `run` should become a
separate explicitly authorized command instead.

### Context

Command context carries authenticated caller/organization identity, the active
permission resolver, storage, feature flags, and cross-cutting facilities
needed by domain orchestration. The exact dependency set changes as domains
evolve and is intentionally not listed here.

Context construction is centralized per transport. Tests use the supported
minimal test constructor rather than assembling partial production context.

Domain-specific algorithms remain in their domain; adding every helper as a
context field would recreate the old service layer.

### Metadata and schema

Static metadata drives MCP and gRPC discovery and contributes protocol
instrumentation. The registered command name is the stable operation identity.
HTTP method and path metadata describe the corresponding adapter but do not
replace the OpenAPI handler annotation.

Input schema is derived from the command type. Commands should declare useful
output schema and shape when callers need to script the result. Generic
discovery must always return a valid schema, even while a domain is being
migrated.

The source registry is authoritative for current commands. Do not maintain a
parallel catalog.

The `/mcp` endpoint and `platform` capability are adapters over the same command
surface. MCP may accept an organization selector because it is a stateless
external protocol. The capability must not: its organization and caller are
re-established from the active session by the server.

## Errors

Domain errors are protocol-independent and may include a stable code, allowed
recovery actions, and retry guidance. The exact error variants and exhaustive
lower-snake-case kind mapping live in `domains/common.rs`.

HTTP maps errors to RFC 9457 Problem Details and carries safe extensions. MCP
currently emits a stable textual kind plus message for bashkit compatibility.
Internal errors are logged with diagnostic detail but must be redacted before
an untrusted transport response.

Adding an error kind requires updating every exhaustive transport and metric
mapping plus their tests. A copied variant table in this spec would hide that
compiler-enforced work, so it is deliberately omitted.

Classify caller mistakes with typed domain errors. String-pattern
classification exists only for older paths and should not grow when a typed
boundary is available.

## Inventory and dispatch

Commands self-register through inventory. Registration provides metadata,
schemas, and a generic dispatch function. This single registry feeds:

- MCP `discover`, read-only query, and mutation execution;
- generic runtime dispatch;
- internal gRPC command discovery and execution;
- policy-coverage tests.

Duplicate names are invalid. Registration and lookup behavior are tested in the
common domain and transport-specific tests.

### Scripted MCP arguments

The MCP catalog adapts array and object parameters into JSON text for bashkit's
flag parser, then coerces them back before command deserialization. This
translation is generic; individual commands must not add ad hoc parsing for the
same limitation.

A command may opt into one positional argument only when it has one obvious
required identity-like parameter. Read-only exposure defaults from operation
semantics but can be overridden for safe search or preview commands.

MCP failures retain a stable kind prefix. The exact labels and formatting
function live in the catalog source and common error mapping.

## HTTP adapters

HTTP handlers own transport concerns: extractors, OpenAPI annotations, status
and headers, URL decoration, and response serialization. They convert validated
transport input into a domain command and run it through the HTTP dispatcher.

Trivial handlers should use dispatcher helpers so response wrapping and future
HTTP-only cross-cutting behavior stay centralized. A handler must not
reimplement command validation or authorization.

Exact request and response bodies belong to handler types and the generated
OpenAPI export, not this spec.

## gRPC command transport

Internal workers execute registered management operations through the generic
command RPC instead of bespoke messages per CRUD operation. The request carries
an operation name, API version, serialized parameters, and organization scope.
Responses preserve typed domain failure classification.

Workers can discover the command catalog and schema hash. A
non-backward-compatible command contract requires an explicit API-version
decision; changing copied protobuf text in a Markdown file is never sufficient.

Exact messages and size limits belong to the protocol source.

## Query helpers and command composition

Queries are policy-free building blocks for commands and trusted internal
orchestration. They accept explicit organization scope where the data is
org-owned. Exposing a query directly to an external adapter bypasses the domain
contract and is prohibited.

One command may compose another command's execution only after the outer
command has authorized the complete operation. If the inner operation has a
distinct permission or audit meaning, call its public command path instead.

## Cross-cutting behavior

`Command::run` is the protocol-independent chokepoint for:

- permission evaluation;
- command tracing;
- low-cardinality success/failure metrics;
- structured failure logging.

The HTTP dispatcher is the HTTP-only chokepoint. Transport-specific concerns
do not belong in `Command::run`, while policy and business validation do not
belong only in HTTP.

See [`permissions.md`](../security/permissions.md),
[`observability.md`](../operations/observability.md), and
[`prometheus-metrics.md`](../operations/prometheus-metrics.md).

## Adding or migrating a domain

1. Identify the feature owner and keep helpers beneath it.
2. Move canonical request/response types to the domain or adapter that owns
   their contract.
3. Extract reusable persistence helpers without policy.
4. Implement each public operation as a command with metadata, schema, and
   policy.
5. Register the command and call `run` through the HTTP dispatcher.
6. Add focused domain tests and rely on inventory tests for cross-transport
   coverage.
7. Remove old service/catalog/dispatcher copies after all callers converge.

Migration status is visible from `crates/server/src/domains/` and inventory;
this spec intentionally does not freeze directory counts or "current" lists.

## Cross-org resolver exception

`domains/org_resolver.rs` is an inventory-backed resolver registry, not a
command domain. It supports membership-gated direct-link fallback while
ordinary entity APIs remain strictly org-scoped. Its contract and registration
requirements live in [`multitenancy.md`](../security/multitenancy.md).
