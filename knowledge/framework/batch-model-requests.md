---
type: Specification
title: "Framework Provider-Native Batch Requests"
description: "Explicit offline completions, scoped resumable tickets, provider boundaries and uncertain submission recovery."
tags:
  - everruns
  - framework
  - providers
---

# Framework Provider-Native Batch Requests

## Decision

Provider-native batch work begins at the direct Framework completion boundary.
The public [Batch API](../../crates/everruns/src/batch.rs) accepts independent
completions or explicit shared message/configuration values, returns a
credential-free ticket and exposes explicit lifecycle
operations. Existing interactive calls keep their behavior. Batch support is an
optional [provider contract](../../crates/contracts/src/batch.rs), never a
synchronous fallback or a streaming driver wrapper.

The first built-in implementation is native Anthropic Message Batches. It reuses
the Messages encoder so reasoning, tool schemas and message layout have one
owner. Providers must explicitly expose batch support. Wasm and Engine-hosted
batch execution are outside this implementation; the public
[guide](../../docs/framework/batch-model-requests.md) owns the support matrix.

The two input paths share whole-batch validation and ticket creation. Explicit
request lists reuse the provider-message contract already used by core; they do
not introduce a second wire schema or implicitly resolve stored attachments.

## Recovery and isolation

The application owns scheduling, input retention, ticket storage and access
control. Tickets must not contain prompts, credentials, result content or remote
result URLs. They are unsigned routing data rather than authorization. A client
never chooses credentials or endpoints from a ticket: trusted provider assembly
and a stable tenant/account scope bind every operation. Rotating a key for the
same account does not invalidate a ticket; changing tenant, account or endpoint
requires a separately scoped client.

Creation and persistence are not an atomic transaction across the provider and
the application. Ambiguous creation has an explicit uncertain outcome and cannot
be retried automatically. A caller records durable intent before submitting and
reconciles with the provider when acceptance or ticket persistence is lost.
Polling and collecting may repeat with the same ticket. Cancellation is
asynchronous and preserves completed requests. No exactly-once external effects,
automatic tool execution or transparent agent-loop durability is promised.

## Success bars

- Match results by submitted ID, reject missing/duplicate/unknown IDs, and return
  submission order even when the provider returns a different order.
- Separate per-request failures from lifecycle failures, and keep opaque provider
  bodies and credential-bearing URLs out of errors.
- Bound HTTP time and response reads; validate provider IDs before URL assembly
  and collect only from the configured endpoint without following redirects.
- Demonstrate submission, process reconstruction, collection and cancellation in
  runnable real-provider examples, with deterministic tests at the public API.

The [acceptance suite](../../crates/everruns/tests/provider_batch.rs) and
[classification example](../../crates/everruns/examples/batch_classification.rs)
are the executable evidence for the lifecycle and recovery boundary.
