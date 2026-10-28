---
title: Batch Model Requests
description: Submit independent model completions through Anthropic Message Batches, persist a credential-free ticket, and collect results in another process.
---

Use `everruns::batch::Batch` for offline classification, extraction, or evaluation
where latency can be hours. A batch sends independent model requests to a
provider's batch API. It does not start an agent, stream tokens, run tools, or
execute side effects. This is an opt-in **alpha** API; ordinary completions and
sessions keep their existing behavior.

## Supported providers and hosts

| Surface | Support |
| --- | --- |
| Anthropic direct Messages API | Native Rust, `anthropic` Cargo feature; Message Batches submit, poll, collect, cancel |
| Anthropic-compatible proxy | Explicit versioned `base_url`; must implement the Message Batches API, not just Messages |
| OpenAI, OpenRouter, Gemini, Bedrock, ChatGPT and other built-in drivers | Unsupported; returns `BatchError::Unsupported`, with no synchronous fallback |
| Custom providers | Implement the optional `batch::BatchDriver` SPI and expose it through `ChatDriver::batch_driver` |
| Native applications with memory, files, SQLite or another application database | Supported direct API; application owns ticket persistence and scheduling |
| Engine memory/SQLite hosting, `serve`, Platform workers and eval service | No batch scheduling or ticket persistence integration; an embedding native application can separately use the direct API |
| Wasm, Cloudflare Durable Objects and celld | No built-in batch driver in this version |

Install the Framework with the provider feature:

```toml
[dependencies]
everruns = { version = "0.49", features = ["anthropic"] }
serde_json = "1"
tokio = { version = "1", features = ["macros", "rt"] }
```

## Submit now, collect later

The batch owns one configured provider. Supply typed request values directly,
or describe each request with a completion builder. Both paths share validation,
submission and the same resumable tickets.

### Pass message lists directly

`submit_requests` accepts a `Vec<BatchRequest>`. Each value contains its ID,
existing messages, and `LlmCallConfig`, so code that already has a conversation
can submit it without rebuilding it through `.system()` and `.user()` calls.
The client stays available for polling, collection and cancellation.

```rust
use everruns::{
    batch::{Batch, BatchRequest},
    llm::{Message, MessageRole},
    providers::anthropic::Anthropic,
    LlmCallConfig,
};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let mut config = LlmCallConfig::new("claude-sonnet-5");
config.max_tokens = Some(64);
let requests = vec![
    BatchRequest {
        id: "review_001".into(),
        messages: vec![
            Message::text(MessageRole::System, "Classify as positive, negative, or neutral."),
            Message::text(MessageRole::User, "Delivery arrived early."),
        ],
        config: config.clone(),
    },
    BatchRequest {
        id: "review_002".into(),
        messages: vec![
            Message::text(MessageRole::System, "Classify as positive, negative, or neutral."),
            Message::text(MessageRole::User, "The replacement does not turn on."),
        ],
        config,
    },
];
let client = Batch::new(Anthropic::from_env()?, "tenant-42/anthropic-account-7");
let ticket = client.submit_requests(requests).await?;
let saved_ticket = serde_json::to_vec(&ticket)?;
# let _ = saved_ticket;
# Ok(())
# }
```

`everruns::llm::Message` is the provider-message contract used by core. Stored
`everruns_core::RuntimeMessage` records are a different type: core hosts reuse the
existing [message conversion helpers](https://github.com/everruns/everruns/blob/main/crates/core/src/llm_conversions.rs).
For a text-only core conversation:

```rust
use everruns::{batch::BatchRequest, LlmCallConfig};
use everruns_core::{RuntimeMessage, llm_conversions::llm_message_from_message};

let conversation = vec![
    RuntimeMessage::system("Classify the review."),
    RuntimeMessage::user("Delivery arrived early."),
];
let request = BatchRequest {
    id: "review_001".into(),
    messages: conversation.iter().map(llm_message_from_message).collect(),
    config: LlmCallConfig::new("claude-sonnet-5"),
};
// Pass vec![request] to client.submit_requests(...).
# let _ = request;
```

For stored images or files, core hosts use the attachment-aware conversion helper
with their resolved attachments. This direct API does not load session history or
resolve stored file references itself.

All request IDs must be unique, and each request needs messages. The entire
list is validated before submission. Mixing an explicit list with buffered
`.request(...)` builder entries is rejected to avoid silently discarding work.

### Completion builder

Build each completion with a **bare model ID**, then give it a unique request
ID. A completion carrying its own provider is rejected so a batch cannot
silently route it to a different account.

```rust
use everruns::{Model, batch::Batch, providers::anthropic::Anthropic};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let model = Model::from("claude-sonnet-5");
let batch = Batch::new(Anthropic::from_env()?, "tenant-42/anthropic-account-7")
    .request("review_001", model.completion()
        .system("Classify as positive, negative, or neutral. Return one word.")
        .user("Delivery arrived early.").max_tokens(64))?
    .request("review_002", model.completion()
        .system("Classify as positive, negative, or neutral. Return one word.")
        .user("The replacement still does not turn on.").max_tokens(64))?;
let ticket = batch.submit().await?;
// Save immediately in access-controlled durable application storage.
let saved_ticket = serde_json::to_vec(&ticket)?;
# let _ = saved_ticket;
# Ok(())
# }
```

### Resume and collect

In another process, reconstruct the provider from current credentials and use
exactly the same application scope and endpoint. Credential rotation is allowed
within that provider account. The ticket contains a provider job ID, request IDs,
scope, format version and endpoint fingerprint. It contains no credentials,
prompts, result URLs, or results.

```rust
use everruns::{batch::{Batch, BatchStatus, BatchTicket}, providers::anthropic::Anthropic};

# async fn collect(saved_ticket: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
let ticket: BatchTicket = serde_json::from_slice(saved_ticket)?;
let client = Batch::new(Anthropic::from_env()?, "tenant-42/anthropic-account-7");
if client.poll(&ticket).await? == BatchStatus::Ended {
    for item in client.collect(&ticket).await? {
        match item.result {
            Ok(response) => println!("{}: {}", item.id, response.text),
            Err(error) => eprintln!("{}: {error:?}", item.id),
        }
    }
}
// Otherwise save the ticket and schedule another poll, usually ≥60 seconds later.
# Ok(())
# }
```

Collection checks that the job ended and maps results by request ID, returning
them in submission order. Unknown, duplicate or missing IDs fail the whole
collection. A request can succeed while another expires, is cancelled, or fails.
Successful responses retain text, reasoning, function tool calls, finish reasons
and token usage. Tool calls are data; nothing executes them. Validate model output
before consuming it. Anthropic's existing driver does not enforce
`Completion::response_format`; batch submission rejects that option too.

## Runnable examples

Both examples call the real provider and incur its normal batch charges. Set
`ANTHROPIC_API_KEY` locally; do not put it in a ticket or command argument.

**Classification across processes:** submit three reviews, then inspect or cancel
the job and collect validated JSON classifications. Every command uses the same
state directory and `BATCH_SCOPE`. `submit` requires a new directory, writes
`intent.json` before the provider call, and saves `ticket.json` after acceptance.
`collect` writes `results.json` without overwriting an earlier collection.

```bash
export BATCH_SCOPE=tenant-42/anthropic-account-7
cargo run -p everruns --features anthropic --example batch_classification -- submit /tmp/reviews-001
cargo run -p everruns --features anthropic --example batch_classification -- poll /tmp/reviews-001
# Run when the poll says Ended:
cargo run -p everruns --features anthropic --example batch_classification -- collect /tmp/reviews-001
# Or request cancellation, then poll and collect partial results:
cargo run -p everruns --features anthropic --example batch_classification -- cancel /tmp/reviews-001
```

The [classification example](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/batch_classification.rs)
uses `claude-sonnet-5`; set `BATCH_MODEL` on submission to choose another active
Anthropic model. Protect the state directory: intents and results contain user
content even though tickets do not contain prompts.

**Batch eval:** a single process submits an explicit list of message-based
requests for three capital-city cases, polls every
60 seconds, and scores exact-match accuracy, correlating each answer by ID.
This example keeps its ticket only in memory; restarting submits a new job.

```bash
cargo run -p everruns --features anthropic --example batch_eval
```

See the [eval source](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/batch_eval.rs)
and the [example catalog](/framework/examples/).

## Retries, recovery and cancellation

There are no automatic retries, resubmissions, background pollers, or cross-client
batch pools. `BatchError::retryable()` identifies transient failures for retrying
**that operation**. A failed poll or collection can use the same ticket again.
For individual failed or expired items, select the work deliberately and create
a new batch. Retain your source inputs separately; the ticket cannot reconstruct
a prompt.

**Uncertain submission is a separate outcome.** If a submit times out, loses its
response, receives a server error, or returns an invalid acceptance response,
it returns `SubmissionUncertain`. The provider may already be processing it.
Do not automatically call `submit` again. Reconcile the intent and request IDs
with the provider console/API first. If the process crashes between acceptance
and saving the ticket, treat the surviving intent the same way. This API does
not guarantee idempotent creation or exactly-once external effects; it cannot
atomically commit a provider job and your database record.

After reconciliation identifies the accepted job, `client.recover(job_id,
original_request_ids).await?` checks access and returns a ticket without creating
work. Supply IDs from the durable intent, then persist this ticket. The
classification example exposes the same recovery path:

```bash
# Only after identifying this intent's accepted job in the provider console/API:
cargo run -p everruns --features anthropic --example batch_classification -- recover /tmp/reviews-001 msgbatch_YOUR_RECONCILED_JOB
```

`client.cancel(&ticket).await?` requests cancellation of the **whole job**.
Cancellation is asynchronous: keep the ticket, poll until `Ended`, and collect
successes together with cancelled items. Completed or already running requests
may still be billed. Dropping a client, ticket, or pending future does not cancel
an external job. Retrying cancellation with the same ticket is safe; already
ended jobs return `Ended`.

## Limits and isolation

- Anthropic accepts at most 100,000 requests and 256 MiB per job. This driver
  rejects oversized submissions; split them explicitly. IDs are 1–64 ASCII
  letters, digits, underscores or hyphens. `max_tokens` must be positive.
- Jobs can take up to 24 hours. Results are retained by Anthropic for 29 days from batch creation;
  collect them into your own storage before retention expires. Polling is not
  a streaming or interactive response path.
- The client defaults to a 60-second timeout per HTTP request and a 256 MiB
  cap per response. Configure `Batch::timeout` and `max_response_bytes` for your
  workload. Streaming first-token/response limits, request-body capture,
  per-item headers and provider execution options are rejected by this batch
  driver. The timeout controls HTTP work, not the offline job's lifetime.
- Every client uses only its own provider credentials and endpoint. It rejects
  mismatched ticket scope, provider, endpoint, or format before network access.
  Choose a distinct stable scope for every tenant **and provider account**;
  the library cannot infer the account behind an API key. A scope is an
  application guard, not provider authorization. Never accept tenant identity
  or a ticket from an unauthenticated caller as proof of access.
- Tickets are serialized routing data, not signed capabilities. Authorize their
  storage and use through your application. Do not pool work across tenants.
  Provider error bodies and endpoint URLs are excluded from batch errors.
  Result URLs are ignored; collection stays on the configured provider endpoint.
- Usage reports token counts, not a locally estimated bill. Consult the
  provider's [current batch pricing and retention rules](https://platform.claude.com/docs/en/build-with-claude/batch-processing).

For direct interactive calls, see [Direct Model Calls](/framework/direct-model-calls/).
For sessions and their hosting guarantees, see [Hosting](/framework/hosting/).
