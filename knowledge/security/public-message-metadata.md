---
type: Decision
title: "Public Message Metadata"
description: "Allowlisted public annotations, caller-owned custom data and trusted replay boundaries."
tags:
  - everruns
  - security
  - api
---

# Public Message Metadata

Runtime metadata mixes user annotations with execution and provider replay state.
Authorization to view a session is not authorization to receive every internal
metadata key. Public projections are defined in
[`public_metadata`](../../crates/contracts/src/public_metadata.rs); the field
names and nested shapes are owned there rather than duplicated here.

REST message reads, creates and duplicate sends project the stored message only
at the return boundary. JSONL export uses that same message service. Typed event
lists/SSE and Agent API visibility projections use the same public event policy;
Serve and Workers share the JSON equivalent. Trace message previews and inlined
raw-event previews also project before publication. The SDK reviewed envelope
filters provenance; its explicit canonical escape hatch remains trusted.

Unknown historical keys are withheld because ownership cannot be inferred from
the key name or the user role. Caller-owned JSON has an explicit custom namespace.
The public migration contract is in [Events](../../docs/features/events.md).
No storage migration is needed: internal journals, database events, gRPC and
recovery keep their complete state, including tool changes and script stdin.

This boundary does not turn generation diagnostics into a confidential surface.
Session-authorized control-plane callers and full-visibility Agent API callers
can intentionally inspect prompts, tools and captured requests. Trace summaries
may show tool names. External actor metadata and tool inputs/results are separate
reviewed contracts; this projection must not recursively delete similarly named
fields inside user content. ATIF exports construct their own trajectory and do
not copy runtime message metadata.

The source audit establishes exposure to callers authorized for the session,
including restricted Agent API callers via create responses, not anonymous or
cross-tenant access. It does not establish which production records contained
secrets or who retrieved them. No production records were queried. Rolling out
all API-serving binaries filters historical reads immediately; old downloads,
client caches, external tracing and backups need separate retention/incident
assessment if sensitive values were present.

Regression coverage includes malformed nested annotations, unknown-key canaries,
typed and JSON event equivalence, replay preservation, both historical storage
formats, create/retry/list, SSE frames, trace previews, JSONL and visibility modes.
