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

Built-in implementations cover Anthropic Message Batches, OpenAI Responses and
Chat Completions batches, Gemini Developer API inline batches, Mistral inline jobs, Groq Files/Batches,
and explicitly configured Fireworks dataset jobs. They reuse
the interactive codecs so request layout and response normalization have one
owner. Capability is exposed through the optional driver hook and a local
provider query; it does not promise account/model eligibility. Providers must explicitly expose batch support. Wasm and Engine-hosted
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
asynchronous; collect available completed requests or handle a provider job-level
failure when no per-item output exists. OpenAI's and Groq's staged upload and job creation
are separate effects. Neither ambiguous mutation is replayed or cleaned up
automatically, since a staging file can already belong to a live job.
Partial OpenAI cancellation uses trusted ticket IDs only after output counts
match; missing successful files must never become invented cancelled rows. No exactly-once external effects,
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

The [Anthropic acceptance suite](../../crates/everruns/tests/provider_batch.rs),
[multi-provider suite](../../crates/everruns/tests/provider_batch_multi.rs) and
[classification example](../../crates/everruns/examples/batch_classification.rs)
are the executable evidence for the lifecycle and recovery boundary.

Run-owned pending-response withdrawal (as in
[Pydantic AI's lifecycle correction](https://github.com/pydantic/pydantic-ai/pull/11517))
does not transfer ownership here: no agent run retains the ticket. A successful
submit-now/collect-later job must survive client/future drops. Only an explicit
application decision cancels its whole job; borrowed/shared provider jobs are
never cancelled as cleanup. An abandoned submission future still requires intent
reconciliation because remote acceptance may have happened.

Fireworks requires an explicit account and exact trusted artifact origins. Its
control plane stays on the configured inference origin; signed storage requests
carry no provider authentication and follow no redirects. Artifact origins never
come from tickets or returned job metadata. Dataset creation, upload and job
creation are independent external effects, with no implicit cleanup or replay.
The published Fireworks API exposes deletion without a cancellation contract;
the batch cancellation operation therefore fails explicitly rather than deleting
collectable state. The public guide owns provider-specific limits and partial
result support. Native HTTP peers in the
[additional-provider acceptance suite](../../crates/everruns/tests/provider_batch_additional.rs)
cover these boundaries without paid calls.
