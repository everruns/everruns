---
type: Specification
title: "Evals"
description: "User-facing behavioral evals."
tags:
  - everruns
  - evaluation
---
# Evals

## Purpose

Evals are org-scoped, user-facing behavioral tests. An eval groups curated
cases; each internal case runs against a real Everruns session and records
scores, artifacts, efficiency data, and a link back to the debuggable
conversation.

This spec owns evaluation semantics and product intent. It does not repeat
Rust models, scorer variants, database columns, identifier prefixes, route
tables, or UI component inventories.

## Sources of truth

- [`crates/platform/src/eval.rs`](../../crates/platform/src/eval.rs) owns eval targets,
  statuses, scorer variants, score/result models, summaries, datasets, and
  serialized field shapes.
- [`crates/server/src/domains/evals/`](../../crates/server/src/domains/evals)
  owns validation, commands, limits, execution, import, scoring, datasets, and
  persistence orchestration.
- [`crates/server/src/api/evals.rs`](../../crates/server/src/api/evals.rs) owns
  HTTP routes and OpenAPI annotations.
- [`docs/api/openapi.json`](../../docs/api/openapi.json) is the exact external
  request/response contract.
- [`crates/server/migrations/`](../../crates/server/migrations) owns eval tables,
  indexes, share tokens, and later schema evolution.
- Eval integration tests under [`crates/server/tests/`](../../crates/server/tests)
  are the executable contract for runs, import, sharing, datasets, and org
  isolation.

## Core concepts

### Target

An eval target defines how a case creates a session. It either describes
session setup or references a deployed app.

Target resolution is most-specific first: run override, case override, eval
default, then the organization's default session setup. Validation follows the
same rules as ordinary session creation so evals do not become a bypass around
session policy.

The exact target variants and fields live in `crates/platform/src/eval.rs`.

### Eval

An eval is a named, tagged collection of cases with lifecycle state and optional
default target/model behavior. It is isolated to one organization and follows
the normal archive/delete conventions for building blocks.

### Case

A case defines:

- one or more input messages sent sequentially;
- optional verification messages sent after the main conversation idles;
- scoring rules;
- execution bounds;
- optional session files to collect as named artifacts;
- an optional target override.

Each internal case gets a fresh session. This preserves production behavior and
makes failures inspectable rather than simulating the agent loop in a separate
test harness.

### Run

A run executes all selected cases under one resolved configuration. It records
source, trigger, lifecycle, aggregate metrics, and results.

Internal runs create sessions and score them. External runs are imported
observations and are never re-executed or silently rescored by Everruns.

A run is terminal when completed, failed, or cancelled. Cancellation prevents
new case work and propagates through the durable runner according to current
execution state.

### Result

A result captures one case outcome, including its session when internally
executed, resolved target state, scores, artifacts, timing, token counts,
metadata, and safe error information.

Target information is frozen for audit at run creation. The exact optionality
and storage representation are defined by the current source; consumers must
use the generated API rather than a copied field table.

## Scoring

A scorer returns pass/fail, a normalized value from zero to one, and a
human-readable reason. A case passes only when every required scorer passes.
The case score is the weighted average of scorer values.

The exhaustive scorer set and configuration are defined by the tagged `Scorer`
enum in `crates/platform/src/eval.rs`. Adding a scorer requires:

- a serialized variant and validation;
- execution logic;
- API/OpenAPI exposure;
- focused tests;
- UI authoring/rendering support where applicable.

Do not keep an exhaustive scorer list here. The source currently includes
output, tool-use, filesystem, schema, and citation-oriented scoring families;
the enum and tests are authoritative as that set evolves.

External score write-back is allowed only through the explicit result/run
commands. Provenance belongs in result metadata so a score can be audited
without pretending Everruns computed it.

## Internal execution

Triggering an internal run:

1. validates org scope, limits, target overrides, and selected cases;
2. creates the run and pending results with frozen resolved targets;
3. schedules durable bounded-concurrency case work;
4. creates one session per case;
5. sends conversation and verification messages in order, waiting for idle
   between messages;
6. reads canonical session outcomes and configured artifacts;
7. evaluates scorers and records result metrics;
8. aggregates the terminal run summary.

Eval sessions use ordinary session capabilities, events, authorization, and
resource limits. Eval execution must not call storage or tools through a
special privileged path.

The runner owns exact concurrency, timeout, tag-filter behavior, and error
classification. Operator configuration and defaults live in the eval limits
source rather than this spec.

## Limits

Run creation is bounded by per-organization concurrent-run and per-run case
limits. Limits are checked before fan-out so one request cannot create
unbounded durable work.

Exact environment variable names, defaults, and injectable test configuration
live in [`crates/server/src/domains/evals/limits.rs`](../../crates/server/src/domains/evals/limits.rs).

## External import

Everruns can host and visualize runs produced by external evaluation systems.
Import is org-scoped and permission-gated.

External import semantics are:

- source attribution identifies the external system and run;
- evals and cases use the documented org-local identity strategy;
- publishing the same external run identity is idempotent and replaces its
  prior imported representation;
- transcripts, opaque metrics, and scorer provenance remain attributed data;
- imported cases are not eligible for internal re-execution merely because
  they now appear in Everruns.

Preflight lets optional-feature clients determine whether import is available
before publishing. Exact payloads and routes live in the command source and
OpenAPI.

ATIF case import is a separate operation for turning trajectories into
executable eval cases. See [`atif-adoption.md`](atif-adoption.md).

## Artifacts and datasets

Cases may collect named session files after conversation and verification work.
Run artifact export provides the external evaluation-oriented record format
defined by the command implementation.

Dataset export is durable and may have its own lifecycle and stored body. The
current dataset commands and models are defined in the eval domain and core
model. This spec intentionally does not copy their routes or fields.

See [`dataset-export.md`](dataset-export.md) for trajectory/reward dataset
semantics.

## Public share links

A completed run may be shared through an unguessable read-only token.

- Only a hash is stored; the raw token is returned once.
- Minting replaces prior active access according to the share command.
- Revoked, expired, malformed, and unknown tokens produce a uniform
  non-enumerating response.
- Public DTOs omit organization internals, private session/app targets,
  environment labels, and other non-public metadata.
- The token is authorization for the public read; management still requires
  normal eval permission.

Exact token format, lifecycle columns, public DTO, and routes belong to source,
migrations, and OpenAPI. Public-boundary rules also follow
[`public-endpoints.md`](../execution/public-endpoints.md).

## UI intent

The product supports:

- browsing active and archived evals;
- authoring cases, targets, messages, artifacts, and scorers;
- triggering and cancelling runs;
- inspecting aggregate and per-case outcomes;
- navigating from an internal result to its session;
- viewing imported and shared results without implying they were executed
  locally;
- comparing runs and identifying regressions.

Exact page structure, component names, table columns, and navigation placement
belong to the UI source and design system.

## Security and isolation

- Every eval, case, run, result, artifact, dataset, and management token is
  scoped to its owning organization.
- Internal execution uses ordinary session permissions and cannot smuggle
  capabilities through a target.
- Public shares expose only sanitized read-only data.
- Raw share tokens and external credentials are never logged or persisted.
- Imported attribution is untrusted display data and must be sanitized.
- Artifact paths are resolved through the session filesystem boundary.
- Scorer failures do not expose internal provider or storage errors to public
  consumers.
