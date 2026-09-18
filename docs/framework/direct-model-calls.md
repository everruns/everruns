---
title: Direct Model Calls
description: Call a model once through the Framework's provider edge, without building an agent.
---

Some work is one prompt and one answer: classify a string, draft a summary,
extract a field. That needs the provider edge — drivers, endpoints, credentials,
retries, error classification — but none of the agent loop around it.

`Model::complete` is the whole API for that case:

```rust
use everruns::{Model, OpenAI};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let model = Model::new("gpt-5.6-terra", OpenAI::from_env()?);
let answer = model.complete("Name the three primary colors.").await?;
println!("{answer}");
# Ok(())
# }
```

The model is the same value an [agent](/framework/agents/) takes, reached
through the same [`Provider`](/framework/models-and-providers/) — which can also
be asked [which models it offers](/framework/model-catalogs/). Nothing is
persisted: a direct completion owns no session, no history, and no workspace.
Reach for an agent as soon as the work needs tools, multiple turns, durability,
or events.

## Testing without a provider

`Model::simulated` returns canned responses from an in-process simulator
(`everruns-llmsim`). It is a **test double**, not a local model: it runs no
inference and is not a way to use Everruns without a model provider. It exists
so tests and examples can assert on agent behavior without a network call or an
API key.

Real work always goes through a provider — see
[Supported providers](/framework/supported-providers/).

```rust
use everruns::Model;

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let answer = Model::simulated("4").complete("What is 2 + 2?").await?;
assert_eq!(answer, "4");
# Ok(())
# }
```

See [Testing and simulation](/framework/testing-and-simulation/) for scripted
multi-response simulators.

## System messages, context, and controls

`Model::completion` describes the call before sending it. Messages append in
call order; each control maps to one provider request field and stays unset
unless assigned, so the provider keeps its own defaults.

```rust
use everruns::{Model, ReasoningEffort};

# async fn run(model: Model) -> Result<(), Box<dyn std::error::Error>> {
let response = model
    .completion()
    .system("Answer with a single word.")
    .user("What is the capital of France?")
    .max_tokens(16)
    .reasoning_effort(ReasoningEffort::Low)
    .send()
    .await?;

println!("{}", response.text);
println!("{:?} tokens", response.metadata.total_tokens);
# Ok(())
# }
```

`send` returns the full `LlmResponse` — text, reasoning artifacts, tool calls,
and call metadata. `text()` returns only the answer text. Replay prior turns
with `.assistant(...)`: the completion carries no history of its own, so
context is whatever the call passes.

When the model is a bare provider-visible id, attach the provider on the
completion instead of the model:

```rust
use everruns::{Model, OpenAI};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let answer = Model::from("gpt-5.6-terra")
    .completion()
    .provider(OpenAI::from_env()?)
    .user("Summarize this in one line: ...")
    .text()
    .await?;
# let _ = answer;
# Ok(())
# }
```

## Streaming

`stream()` returns the provider's events as they arrive, ending with a `Done`
event carrying the call's metadata:

```rust
use everruns::{LlmStreamEvent, Model};
use futures::StreamExt;

# async fn run(model: Model) -> Result<(), Box<dyn std::error::Error>> {
let mut stream = model.completion().user("Write a haiku.").stream().await?;
while let Some(event) = stream.next().await {
    if let LlmStreamEvent::TextDelta(delta) = event? {
        print!("{delta}");
    }
}
# Ok(())
# }
```

## Errors

`CompletionError` separates configuration mistakes from provider failures:

- `MissingProvider` — the model names an id but nothing says how to reach it.
- `NoMessages` — the completion was sent empty.
- `Call(..)` — the provider call failed, carrying the `AgentLoopError` and its
  full `LlmError` classification.

The first two are caught before any request leaves the process.

## Going lower

`Completion` is a thin value-first layer over `Provider`, which is public.
Applications that already hold a `Provider` — or implement their own
[`ChatDriver`](/framework/custom-providers/) — can call it directly with
`everruns::llm`'s `Message` and `MessageRole`, plus the crate-root
`LlmCallConfig` and `LlmResponse` re-exports:

```rust
use everruns::llm::{Message, MessageRole};
use everruns::{LlmCallConfig, Provider};

# async fn run(provider: Provider) -> Result<(), Box<dyn std::error::Error>> {
let response = provider
    .chat_completion(
        vec![Message::text(MessageRole::User, "What is 2 + 2?")],
        &LlmCallConfig::new("gpt-5.6-terra"),
    )
    .await?;
# let _ = response;
# Ok(())
# }
```

That surface is the driver boundary itself: every field of `LlmCallConfig`,
including tool definitions, is available, and nothing is defaulted for you.

The runnable version of this page is
[`direct_llm.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/direct_llm.rs),
which runs offline without an API key.
