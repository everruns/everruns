---
title: Batch Model Requests
description: Submit independent model completions through native provider batch APIs, persist a credential-free ticket, and collect results in another process.
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
| OpenAI direct Responses API | Native Rust, `openai` feature; Files upload and Batches submit, poll, collect, cancel |
| OpenAI direct Chat Completions API | Same lifecycle, using `OpenAI::from_env()?.api(OpenAIApi::ChatCompletions)`; tickets are separate from Responses tickets |
| Gemini Developer API | Native Rust, `gemini` feature; inline batch submit, poll, collect, cancel; Files and Vertex AI modes are unsupported |
| Mistral La Plateforme | Native Rust, `mistral` feature; inline request submission, polling, inline/file result collection and cancellation |
| Groq Chat Completions | Native Rust, `groq` feature; Files/Batches submit, poll, collect and cancel; audio batches are outside this completion API |
| Fireworks | Native Rust, `fireworks` feature; explicit account and artifact policy; dataset upload, native batch job submission, poll and collect. Cancellation returns `UnsupportedOperation` |
| Custom endpoints | Must implement the complete corresponding native batch API; a chat-compatible endpoint alone is insufficient. Azure batch configurations are not qualified here |
| OpenRouter, Bedrock, ChatGPT and remaining built-in drivers | Unsupported; returns `BatchError::Unsupported`, with no synchronous fallback |
| Legacy endpoint-bound `into_boxed_driver()` / `DriverRegistry::create_chat_driver()` adapters | No batch hook: retain the configured `Provider` for ticket endpoint binding instead. Registry request-option/credential wrappers also fail closed |
| Custom providers | Implement the optional `batch::BatchDriver` SPI and expose it through `ChatDriver::batch_driver` |
| Native applications with memory, files, SQLite or another application database | Supported direct API; application owns ticket persistence and scheduling |
| Engine memory/SQLite hosting, `serve`, Platform workers and eval service | No batch scheduling or ticket persistence integration; an embedding native application can separately use the direct API |
| Wasm, Cloudflare Durable Objects and celld | No built-in batch driver in this version |

The batch API is currently on `main` and is not included in the published
`0.49.0` crate. Until a crate release includes it, use `main` with
the provider features you need. Commit `Cargo.lock` to retain the resolved Git
revision for reproducible builds:

```toml
[dependencies]
everruns = { git = "https://github.com/everruns/everruns", branch = "main", features = ["anthropic"] }
serde_json = "1"
tokio = { version = "1", features = ["macros", "rt"] }
```

For Core hosts using the stored-message conversion example, source `everruns-core`
from the same branch and enable `host`, so both crates share the message contract:

```toml
[dependencies.everruns-core]
git = "https://github.com/everruns/everruns"
branch = "main"
default-features = false
features = ["host"]
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

## Choose a provider through the same API

`Provider::supports_batch()` checks the driver's local capability. It does not
probe model eligibility, account permissions, quotas or a custom server's API.
Unsupported drivers retain the default `ChatDriver::batch_driver() == None`.
Custom drivers implement the same optional SPI; Framework does not dispatch on
provider names.

With the `openai` feature:

```rust
use everruns::{Model, batch::Batch, providers::openai::OpenAI};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let provider: everruns::Provider = OpenAI::from_env()?.into();
assert!(provider.supports_batch());
let client = Batch::new(provider, "tenant-42/openai-account-7");
let ticket = client.request("review_001", Model::from("gpt-6.1-sol")
    .completion().user("Classify: delivery arrived early.").max_tokens(64))?
    .submit().await?;
# let _ = ticket;
# Ok(())
# }
```

For Gemini, enable `gemini`, use `Gemini::from_env()` from
`everruns::providers::gemini`, select a Gemini model and its own tenant/account
scope. `submit_requests(Vec<BatchRequest>)`, tickets and lifecycle operations
stay the same. Requests in each OpenAI, Gemini, Mistral, Groq or Fireworks job must use one model.

To select OpenAI Chat Completions explicitly, configure the connection with its
typed API selector:

```rust
use everruns::{OpenAI, OpenAIApi, Provider};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let provider: Provider = OpenAI::from_env()?
    .api(OpenAIApi::ChatCompletions)
    .into();
assert!(provider.supports_batch());
# Ok(())
# }
```

### Mistral, Groq and Fireworks

Mistral and Groq use the same lifecycle with their own features and provider
values: `Mistral::from_env()` (`MISTRAL_API_KEY`) and `Groq::from_env()`
(`GROQ_API_KEY`). Groq is also a first-class interactive Chat Completions driver,
with model discovery and the shared message/tool codec. Only batch-compatible
models are eligible; driver support does not imply account or model access.

Fireworks separates its inference and management paths and requires an account.
Configure exact trusted storage origins before submitting; obtain your account's
artifact origin from its storage configuration or a trusted existing dataset.
For example, an account whose signed downloads use `storage.googleapis.com`:

```rust
use everruns::{batch::Batch, providers::fireworks::{Fireworks, FireworksBatchConfig}};

# fn example() -> Result<(), Box<dyn std::error::Error>> {
let config = FireworksBatchConfig::new("my-fireworks-account")?
    .artifact_origin("https://storage.googleapis.com")?;
let client = Batch::new(Fireworks::from_env()?.batch(config), "tenant/fireworks-account");
# let _ = client;
# Ok(())
# }
```

Enable `fireworks`. The key comes from `FIREWORKS_API_KEY`; account and artifact
policy stay explicit. Add each exact storage origin your account uses. Downloads
carry only the signed URL, never provider authentication or configured headers;
redirects and untrusted origins fail closed. A custom endpoint must serve both
management `/v1` and inference `/inference/v1` paths on the configured origin.
Changing the account requires a separate client even if its tenant scope was
accidentally reused.

Fireworks' published API offers job deletion rather than a cancellation contract.
`cancel` returns `BatchError::UnsupportedOperation`; it never deletes jobs or
datasets. Manage deletion explicitly in the provider console after preserving
needed results. Failed/cancelled/expired Fireworks jobs return `JobFailed`; this
version does not collect partial datasets from those job states.

### Provider limits and terminal results

| Driver | Submission boundary | Collection and cancellation |
| --- | --- | --- |
| Anthropic | 100,000 requests, 256 MiB | Native result for every request; completed work remains collectable after cancellation |
| OpenAI Responses / Chat Completions | 50,000 requests, 200 MB including this implementation's small multipart envelope; one model | Merges output and error JSONL by `custom_id`. Cancelled/expired jobs may omit unprocessed requests: ticket IDs and validated provider counts identify these, preserving completed results |
| Mistral | Inline submission only, at most 10,000 requests and 20 MB encoded body; one model, 24-hour job timeout | Native inline results or output/error files, correlated by `custom_id`; cancelled/expired omissions require verified counts and original ticket IDs |
| Groq | 50,000 requests, 200 MB including the multipart envelope; one model, 24-hour processing window | Native output/error files and verified count mapping preserve completed requests on cancellation or expiry |
| Fireworks | At most 50,000 requests and 100 MB including the multipart envelope; one model; provider default job window | Downloads up to 16 signed artifacts with an aggregate configured response-byte budget. Missing/duplicate/unknown request IDs fail closed |
| Gemini Developer API | Inline only, 20 MB encoded body, at most 100,000 requests; one model | Maps `metadata.key`, including inline per-item errors. A failed/cancelled job without inline outputs returns `BatchError::JobFailed`; missing outputs are never invented |

Account/model quotas can impose lower limits. Split jobs explicitly. Gemini's
Files mode and Vertex storage outputs are outside this implementation.
OpenAI global validation failures also return `JobFailed`, separate from a list
of per-item failures. Missing successful/error files or inconsistent counts fail
collection rather than being relabelled as cancellation.

OpenAI and Groq submission upload a `purpose=batch` file before creating a job. These
are separate effects. An uncertain upload or job creation returns
`SubmissionUncertain`; a definite rejection can still leave an uploaded file.
The library does not automatically delete input/output files, cancel shared jobs,
or follow provider-supplied URLs. Reconcile files and jobs in your own account,
apply your retention policy and collect results before provider retention expires.
Fireworks dataset creation, upload and job creation are separate external effects
too. No stage is automatically replayed or deleted, including staging artifacts
left after rejection. Persist intent before submitting and reconcile ambiguous
responses through provider job/dataset listings before trying again. Mistral
inline submission has one create operation; an ambiguous response still needs
reconciliation. Provider retention and account quotas apply to all artifacts.

See the current [Mistral batch lifecycle](https://docs.mistral.ai/studio/batch-processing),
[Groq batch lifecycle](https://console.groq.com/docs/batch),
[Fireworks batch lifecycle](https://docs.fireworks.ai/guides/batch-inference),
[OpenAI batch lifecycle](https://developers.openai.com/api/docs/guides/batch)
and [Gemini batch lifecycle](https://ai.google.dev/gemini-api/docs/batch-api).

OpenAI batch function schemas explicitly use non-strict arguments, preserving
optional properties without retaining original schemas in tickets. Structured
text output remains available through OpenAI's response format. Gemini batch
rejects response formats, metadata, speed and verbosity options this encoder does
not support. Stateful continuations, tool search, provider execution options and
per-item credential/header overrides are rejected before submission. Function
tool calls are returned as data, with no provider-hosted tool promotion.

## Runnable examples

Run these examples from a repository checkout containing this change. Both
examples call the real provider and incur its normal batch charges. Set the
provider credential locally; do not put it in a ticket or command argument.

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

Select a provider with `BATCH_PROVIDER` and enable its feature. Keep that selection,
`BATCH_SCOPE` and state directory identical on later invocations:

```bash
# Set OPENAI_API_KEY locally first; each command can incur provider charges.
export BATCH_PROVIDER=openai
export BATCH_SCOPE=tenant-42/openai-account-7
cargo run -p everruns --features openai --example batch_classification -- submit /tmp/openai-reviews-001
cargo run -p everruns --features openai --example batch_classification -- poll /tmp/openai-reviews-001
cargo run -p everruns --features openai --example batch_classification -- collect /tmp/openai-reviews-001

# Gemini uses GEMINI_API_KEY (or GOOGLE_API_KEY).
BATCH_PROVIDER=gemini cargo run -p everruns --features gemini --example batch_eval
# Mistral and Groq use MISTRAL_API_KEY and GROQ_API_KEY respectively.
BATCH_PROVIDER=mistral cargo run -p everruns --features mistral --example batch_eval
BATCH_PROVIDER=groq cargo run -p everruns --features groq --example batch_eval
# Fireworks also needs explicit account and trusted storage origin.
export FIREWORKS_ACCOUNT_ID=my-fireworks-account
export FIREWORKS_ARTIFACT_ORIGIN=https://storage.googleapis.com
BATCH_PROVIDER=fireworks cargo run -p everruns --features fireworks --example batch_eval
```

The [classification example](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/batch_classification.rs)
defaults to `claude-sonnet-5`, `gpt-6.1-sol` or `gemini-4-argon` for the selected
provider; set `BATCH_MODEL` on submission to choose another eligible model. Protect the state directory: intents and results contain user
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
available successes together with cancelled items, or handle the provider
job-level failure described above. Completed or already running requests
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
  Result URLs are ignored except for Fireworks signed artifacts: those are
  fetched only from explicitly trusted origins with no provider credentials.
  Job operations stay on the configured provider origin.
- Usage reports token counts, not a locally estimated bill. Consult the
  provider's [current batch pricing and retention rules](https://platform.claude.com/docs/en/build-with-claude/batch-processing).

For direct interactive calls, see [Direct Model Calls](/framework/direct-model-calls/).
For sessions and their hosting guarantees, see [Hosting](/framework/hosting/).
