---
title: Direct Model Calls and Decisions
description: Call a model once through the Framework's provider edge, or ask Decisions for a number instead of prose, without building an agent.
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
through the same [`Provider`](/framework/models-and-providers/), which can also
be asked [which models it offers](/framework/models-and-providers/#model-catalogs). Nothing is
persisted: a direct completion owns no session, no history, and no workspace.
Reach for an agent as soon as the work needs tools, multiple turns, durability,
or events.

When the answer should be a number rather than prose, such as a probability, a
choice among options, or a graded score, use [Decisions](#decisions) instead.

## Testing without a provider

`Model::simulated` returns canned responses from an in-process simulator
(`everruns-llmsim`). It is a **test double**, not a local model: it runs no
inference and is not a way to use Everruns without a model provider. It exists
so tests and examples can assert on agent behavior without a network call or an
API key.

Real work always goes through a provider — see
[Supported providers](/framework/models-and-providers/#supported-providers).

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

What the stream guarantees:

- Every `TextDelta` carries text. Assistant text is their concatenation.
- `ReasoningDelta` streams reasoning live; the `ReasoningItem` that follows
  repeats the whole block, plus any replay state. Show the deltas and store
  the item, or use the item alone; adding both duplicates the reasoning.
- The stream ends with exactly one `Done` or `Error`. `Error` keeps the
  provider's message, error code, and HTTP status, including errors a gateway
  reports inside a `200` stream.
- `Done` carries `finish_reason` as the provider sent it, or `None` when it
  sent none. It is never filled in as `stop`, so a caller can treat a missing
  reason as a failure.
- Drivers retry `429` and transient `5xx` responses before the first event:
  up to 2 retries and 30 seconds by default. That time counts against any
  timeout you put around the call. To own retries yourself, build the driver
  with `.with_retry_config(everruns::llm::LlmRetryConfig::no_retry())`.

The full contract is on `LlmStreamEvent` in the API reference.

## Errors

`CompletionError` separates configuration mistakes from provider failures:

- `MissingProvider` — the model names an id but nothing says how to reach it.
- `NoMessages` — the completion was sent empty.
- `Call(..)` — the provider call failed, carrying the `AgentLoopError` and its
  full `LlmError` decision.

The first two are caught before any request leaves the process.

## Going lower

`Completion` is a thin value-first layer over `Provider`, which is public.
Applications that already hold a `Provider` — or implement their own
[`ChatDriver`](/framework/models-and-providers/#custom-providers) — can call it directly with
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

When all you have is a base URL, `DriverId::for_base_url` names the driver
whose vendor serves it (OpenAI, Azure OpenAI, OpenRouter, Anthropic, Gemini,
Fireworks). It returns `None` for a gateway or self-hosted server; those speak
the OpenAI-compatible wire, so use `DriverId::OpenAICompletions`.

The runnable version of this section is
[`direct_llm.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/direct_llm.rs),
which runs offline without an API key.

## Decisions

Some questions have typed answers. *Is this claim supported by the source? How
severe is this complaint? Which queue does this ticket belong in?* A chat model
answers those in prose, so the call site ends up with a prompt asking for JSON,
a parser, and a fallback for when the parse fails.

A decision service answers them as numbers instead, and the decision stays in your
code.

This is the counterpart to a direct model call: the same shape, a different
contract.

| | `Model` | `Decisions` |
|---|---|---|
| you send | messages | state plus typed questions |
| you get back | text | calibrated numbers |
| decides the outcome | the model's words | your threshold, in your code |
| streams | yes | no — one round trip |

### Quick start

```bash
cargo add everruns --features typesafe
cargo add tokio --features macros,rt-multi-thread
export TYPESAFE_API_KEY=...   # a key from typesafe.ai
```

```rust
use everruns::{Decisions, TypeSafeAI};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let decisions = Decisions::new("jev-latest", TypeSafeAI::from_env()?);

    let text = "CONGRATULATIONS! You've WON $1,000,000. Click here to claim your prize now!";
    let spam = decisions.probability("Is this message spam?", text).await?;

    println!("spam: {spam:.2}");
    if spam > 0.9 {
        println!("quarantined");
    }
    Ok(())
}
```

```text
spam: 0.98
quarantined
```

One number, and your own `> 0.9` decides — the model reports how likely, not what
to do. The same call answers `0.03` for "Standup moved to 10am." and `0.74` for a
bare "Claim your prize now!"; the middling ones are what a threshold is for.

`--features typesafe` adds `TypeSafeAI` and the `Jev` capability. Without it
`everruns` names no vendor: `Decisions::new` takes any `DecisionsService`.

### OpenAI's Decisions API

[OpenAI's Decisions API](https://developers.openai.com/api/docs/guides/decisions)
(public beta, model `gpt-6-luna`) answers the same questions. `OpenAIDecisions`
in `everruns-integrations` is a `DecisionsService`, so it drops into the same
`Decisions`:

```bash
cargo add everruns
cargo add everruns-integrations --features openai-decisions
export OPENAI_API_KEY=...
```

```rust
use everruns::Decisions;
use everruns_integrations::openai_decisions::OpenAIDecisions;

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let service = OpenAIDecisions::new(std::env::var("OPENAI_API_KEY")?);
let decisions = Decisions::new("gpt-6-luna", service);

let urgent = decisions
    .probability("Does this convey urgency?", "My card was charged twice, fix it today.")
    .await?;
println!("urgent: {urgent:.2}");
# Ok(())
# }
```

A runnable version that routes a support ticket with all three primitives is
[`openai_decisions_triage`](https://github.com/everruns/everruns/blob/main/crates/integrations/examples/openai_decisions_triage.rs):

```bash
OPENAI_API_KEY=... cargo run -p everruns-integrations --features openai-decisions --example openai_decisions_triage
```

All three primitives are native and calibrated, and one request sends every
question in one call. Score questions take at most ten levels. On Everruns
Platform, set `UTILITY_DECISION_DRIVER=openai` with `UTILITY_OPENAI_API_KEY` to answer
guardrail `jev` checks with it (see
[environment variables](/sre/environment-variables/)).

### Three primitives

A decision asks one or more questions about the same state. Each is one of
three shapes:

```rust
use everruns::Decisions;

# async fn run(decisions: Decisions) -> Result<(), Box<dyn std::error::Error>> {
let answers = decisions
    .about("I've been on hold for two hours and my card was charged twice.")
    .noul("urgent", "Does this convey urgency?")
    .score(
        "severity",
        "How severe is the problem the writer describes?",
        [
            "A minor annoyance",
            "A real problem with their account",
            "Serious harm requiring immediate action",
        ],
    )
    .choice(
        "queue",
        "Which team should handle this message?",
        ["billing", "technical", "sales"],
    )
    .send()
    .await?;

let urgent: f64 = answers.probability("urgent")?;
let queue: &str = answers.selected("queue")?;
# Ok(())
# }
```

- **`noul`** — whether something holds, as the probability of yes. A value near
  0.5 means yes and no are near-equally likely, not "medium".
  ([Noul](https://docs.typesafe.ai/primitives/noul))
- **`choice`** — exactly one option from your set, with the distribution behind
  it. Needs at least two options.
  ([Choice](https://docs.typesafe.ai/primitives/choice))
- **`score`** — a position along levels you define, lowest first. Needs at least
  two levels. ([Score](https://docs.typesafe.ai/primitives/score))

The three are System One's own, so TypeSafe's
[Primitives](https://docs.typesafe.ai/primitives) documents what each answer
means and how to choose between them, and
[State](https://docs.typesafe.ai/concepts/state) covers what to put in the
`about(...)` value. Everruns names them the same way rather than inventing
synonyms.

Questions in one call are answered **in parallel inside a single request**, so
asking five costs one round trip, not five. TypeSafe calls leaning on that
[speculative fan-out](https://docs.typesafe.ai/patterns/fan-out): ask the
questions you *might* need, and let your code decide which ones mattered.

### Ids are yours; instructions are the model's

The id labels the answer for your code and is never sent to the model. A
question whose meaning lives in its id asks nothing:

```rust ignore
// Wrong: the model never sees "is_the_joke_funny".
.noul("is_the_joke_funny", "?")

// Right: the instructions carry the question.
.noul("funny", "Would a general audience laugh at this joke?")
```

The same applies to score levels. Describe concrete situations — "A minor
annoyance" reads on its own where "2 out of 5" does not.

### Read the tail, not the average

For "is there any serious hit here" rules, read the probability mass at or above
a level rather than the weighted score. Something probably fine but possibly
awful must not average into fine:

```rust
# async fn run(answers: everruns::Answers) -> Result<(), Box<dyn std::error::Error>> {
// Not: answers.score("severity")? > 1.5
let serious = answers.tail("severity", 2)?;
if serious > 0.3 {
    println!("escalated");
}
# Ok(())
# }
```

### Errors

`DecisionsError` separates configuration mistakes from service failures, the
same split [`CompletionError`](#errors) makes:

- `MissingService` — the decisions was built without a service to reach.
- `NoQuestions` — the decision was sent with nothing to ask.
- `Unconfigured` — the service exists but the deployment never configured its
  credential, so it would answer nothing.
- `NoSuchAnswer(id)` — you read an id that was not asked, or read an answer as
  the wrong shape (a `score` as a probability).
- `Call(..)` — the service call failed, carrying the `AgentLoopError`.

The first three are caught before any request leaves the process.
`Unconfigured` is worth handling separately: a guardrail treats it as fail-open,
but a direct caller usually wants to know the number never arrived rather than
read a confident-looking default.

### Going lower

`Decision` is a thin value-first layer over `DecisionsService`, which is
public. Applications that already hold a service — or implement their own, over
a different vendor or a local model — can call it directly with `everruns`'s
`DecisionRequest`, `DecisionQuestion`, and `DecisionAnswer`
re-exports:

```rust
use everruns::{DecisionQuestion, DecisionRequest, DecisionsService};

# async fn run(service: std::sync::Arc<dyn DecisionsService>) -> Result<(), Box<dyn std::error::Error>> {
let outcome = service
    .evaluate(
        DecisionRequest::new("Claim your prize now!")
            .ask("spam", DecisionQuestion::noul("Is this message spam?")),
    )
    .await?;
# let _ = outcome;
# Ok(())
# }
```

That surface is the contract itself: every question type and the full
`DecisionOutcome`, including usage, with nothing defaulted for you.
Implementing `DecisionsService` is also how a different decision service (another
vendor, or a fine-tuned local model) plugs into the same `Decisions`,
guardrails included.

### Choosing a model

The model is named up front, the way `Model::new`
names one: the service is transport, and the model is the thing that answers.
There is no default to inherit without noticing, because a threshold calibrated
against one version is not evidence about the next.

Ids are the provider's own, so they are spelled the way the vendor spells them.
`jev-latest` is TypeSafe's alias for the current Jev, so it tracks whatever the
current version is; an exact id like `jev-1.13.0` pins one, so a vendor update
cannot move your thresholds under you. Bare `jev` is not an id the API knows —
nothing here rewrites what you pass.

Ask for the alias and read back what answered, which is the id to pin once a
threshold is calibrated:

```rust
# use everruns::Answers;
# fn run(answers: Answers) {
let version = answers.model(); // "jev-1.13.0" for a "jev-latest" request
# let _ = version;
# }
```

A single call can name a different model with the same method on the request
builder, and it wins for that call:

```rust
# use everruns::Decisions;
# async fn run(decisions: Decisions) -> Result<(), Box<dyn std::error::Error>> {
let answers = decisions
    .about("...")
    .noul("urgent", "Does this convey urgency?")
    .model("jev-1.13.0")
    .send()
    .await?;
# let _ = answers;
# Ok(())
# }
```

A deployment that must pin a model does so by never exposing the knob in the
config an agent writes — not by the type being unable to carry one, because
there will be other decision services and other models.

A deployment running the Everruns platform configures a separate
`UTILITY_TYPESAFE_API_KEY` for its [guardrails](/capabilities/guardrails/) — a
different account from the one an embedding application holds.

### Giving an agent the decisions

Everything above is agentless: your code asks, your code decides. The other
half is letting an *agent* classify as part of its own work — checking a claim
against a source before citing it, rating a draft before sending it.

`Jev` is the same decisions as a capability, so an agent gets it as a tool:

```rust
use everruns::{Agent, Engine, Jev, Model, OpenAI};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .name("reviewer")
    .instructions(
        "You review copy. When asked how something reads, measure it with \
         jev_decision and report the numbers rather than judging by eye.",
    )
    .model(Model::new("gpt-5.6-terra", OpenAI::from_env()?))
    .capability(Jev::from_env()?)
    .build()?;

let session = Engine::new().create(agent);
let turn = session
    .run("Rate this subject line for pushiness: 'Act now before it is too late'")
    .await?;
# Ok(())
# }
```

The agent calls `jev_decision`, writing its own questions about whatever it is
looking at, and gets the same calibrated numbers back. It is the identical tool
the hosted [TypeSafe integration](/integrations/typesafe/) gives platform
agents — same name, same schema — so behavior matches whether you embed the
Framework or run on Everruns.

Which one to reach for:

| | you decide | the agent decides |
|---|---|---|
| **who asks** | your code writes the questions | the model writes the questions |
| **use** | `Decisions` | the `Jev` capability |
| **good for** | a policy check, a routing rule, a gate | verification inside a longer task |

### What stays with an agent

A decision owns no session, no history, and no workspace, and runs no
tools.
Reach for an [agent](/framework/agents/) as soon as the work needs any of those.
Typed output guarantees the interface, not the truth: validate thresholds
against your own data and consequences.

### Testing without a credential

`Decisions::simulated` returns a fixed number from an in-process stub. It is a
**test double**, not a local decisions: it runs no inference, reads nothing
from the state you pass it, and is not a way to classify without a provider. It
exists so tests and examples can assert on the code around a decision
without a network call or an API key.

Real work always goes through a decision service: `TypeSafeAI` above, or
your own `DecisionsService`.

```rust
use everruns::Decisions;

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let p = Decisions::simulated(0.93)
    .probability("Does this convey urgency?", "Two hours on hold.")
    .await?;
assert!(p > 0.9);
# Ok(())
# }
```

Because the answer is fixed, a simulated decision proves your threshold
logic runs — never that a real decision service would return that number.

The runnable version of this section is
[`direct_decisions.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/direct_decisions.rs).
It uses the stub by default so it runs with no key; pass `--live` (with
`--features typesafe` and `TYPESAFE_API_KEY` set) to send the same questions to a
real decision service. [`agent_decisions.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/agent_decisions.rs)
does the same for the agent path.

### Shared provider accounts for decisions

`Decisions::from_registry` accepts a `ProviderRegistry` and an explicit `ModelSpec`.
Select `ModelSpec::on("typesafe", "jev-1.13.0")` for direct TypeSafe, or
`ModelSpec::on("openrouter", "jev-1.13.0")` for OpenRouter. The OpenRouter driver maps that
explicit alias to `typesafe/jev-1.13`; other IDs remain opaque. Providers hold authentication
once, so their chat and decision drivers use the same account. Returned model IDs report
the snapshot that actually answered.

`Jev::with_provider(provider, model_spec)` supplies that same account to a Framework capability.
Use its exact provider key in the model spec. Existing `Jev::new` and `Jev::with_client` conveniences
remain available for applications holding a direct TypeSafe key.
