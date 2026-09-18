---
title: Direct Classification
description: Ask a classifier for a number instead of prose, without building an agent.
---

Some questions have typed answers. *Is this claim supported by the source? How
severe is this complaint? Which queue does this ticket belong in?* A chat model
answers those in prose, so the call site ends up with a prompt asking for JSON,
a parser, and a fallback for when the parse fails.

A classifier answers them as numbers instead, and the decision stays in your
code:

```rust
use everruns::{Classifier, TypeSafe};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let classifier = Classifier::new(TypeSafe::from_env()?);
let spam = classifier
    .probability("Is this message spam?", "Claim your prize now!")
    .await?;
if spam > 0.9 {
    println!("quarantined");
}
# Ok(())
# }
```

This is the counterpart to [direct model calls](/framework/direct-model-calls/):
the same shape, a different contract.

| | `Model` | `Classifier` |
|---|---|---|
| you send | messages | state plus typed questions |
| you get back | text | calibrated numbers |
| decides the outcome | the model's words | your threshold, in your code |
| streams | yes | no — one round trip |

## Three primitives

A classification asks one or more questions about the same state. Each is one of
three shapes:

```rust
use everruns::Classifier;

# async fn run(classifier: Classifier) -> Result<(), Box<dyn std::error::Error>> {
let answers = classifier
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
- **`choice`** — exactly one option from your set, with the distribution behind
  it. Needs at least two options.
- **`score`** — a position along levels you define, lowest first. Needs at least
  two levels.

Questions in one call are answered **in parallel inside a single request**, so
asking five costs one round trip, not five.

## Ids are yours; instructions are the model's

The id labels the answer for your code and is never sent to the model. A
question whose meaning lives in its id asks nothing:

```rust
// Wrong: the model never sees "is_the_joke_funny".
.noul("is_the_joke_funny", "?")

// Right: the instructions carry the question.
.noul("funny", "Would a general audience laugh at this joke?")
```

The same applies to score levels. Describe concrete situations — "A minor
annoyance" reads on its own where "2 out of 5" does not.

## Read the tail, not the average

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

## Errors

`ClassifierError` separates configuration mistakes from service failures, the
same split [`CompletionError`](/framework/direct-model-calls/#errors) makes:

- `MissingService` — the classifier was built without a service to reach.
- `NoQuestions` — the classification was sent with nothing to ask.
- `Unconfigured` — the service exists but the deployment never configured its
  credential, so it would answer nothing.
- `NoSuchAnswer(id)` — you read an id that was not asked, or read an answer as
  the wrong shape (a `score` as a probability).
- `Call(..)` — the service call failed, carrying the `AgentLoopError`.

The first three are caught before any request leaves the process.
`Unconfigured` is worth handling separately: a guardrail treats it as fail-open,
but a direct caller usually wants to know the number never arrived rather than
read a confident-looking default.

## Going lower

`Classification` is a thin value-first layer over `ClassifierService`, which is
public. Applications that already hold a service — or implement their own, over
a different vendor or a local model — can call it directly with `everruns`'s
`ClassificationRequest`, `ClassificationQuestion`, and `ClassificationAnswer`
re-exports:

```rust
use everruns::{ClassificationQuestion, ClassificationRequest, ClassifierService};

# async fn run(service: std::sync::Arc<dyn ClassifierService>) -> Result<(), Box<dyn std::error::Error>> {
let outcome = service
    .evaluate(
        ClassificationRequest::new("Claim your prize now!")
            .ask("spam", ClassificationQuestion::noul("Is this message spam?")),
    )
    .await?;
# let _ = outcome;
# Ok(())
# }
```

That surface is the contract itself: every question type and the full
`ClassificationOutcome`, including usage, with nothing defaulted for you.
Implementing `ClassifierService` is also how a different classifier — another
vendor, or a fine-tuned local model — plugs into the same `Classifier`,
guardrails included.

## Credentials

`TypeSafe::from_env()` reads your application's own
`TYPESAFE_API_KEY`, and requires the `typesafe` feature:

```toml
everruns = { version = "0.28", features = ["typesafe"] }
```

`everruns` itself stays vendor-free without that feature: `Classifier::new`
takes any `ClassifierService`, exactly as `Model::new` takes any provider.

## Choosing a model

A service has a default model, so naming one is an override rather than a
required argument — the difference from [`Model::new`](/framework/direct-model-calls/),
where a provider is pure transport and serves many models with no default.

```rust
# use everruns::Classifier;
# fn run(classifier: Classifier) {
let classifier = classifier.model("jev-latest");
# let _ = classifier;
# }
```

Ids are the provider's own, so they are spelled the way the vendor spells them.
`jev-latest` is TypeSafe's alias for the current Jev and is what `TypeSafe` asks
for when you name nothing; an exact id like `jev-1.13.0` pins a version so a
vendor update cannot move your thresholds under you. Bare `jev` is not an id the
API knows — nothing here rewrites what you pass.

A single call can override the model again with the same method on the builder.
A deployment that must pin one does so by never exposing the knob in the config
an agent writes — not by the type being unable to carry one, because there will
be other classifiers and other models.

A deployment running the Everruns platform configures a separate
`UTILITY_TYPESAFE_API_KEY` for its [guardrails](/capabilities/guardrails/) — a
different account from the one an embedding application holds.

## Giving an agent the classifier

Everything above is agentless: your code asks, your code decides. The other
half is letting an *agent* classify as part of its own work — checking a claim
against a source before citing it, rating a draft before sending it.

`Jev` is the same classifier as a capability, so an agent gets it as a tool:

```rust
use everruns::{Agent, Engine, Jev, Model, OpenAI};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let agent = Agent::builder()
    .name("reviewer")
    .instructions(
        "You review copy. When asked how something reads, measure it with \
         jev_evaluate and report the numbers rather than judging by eye.",
    )
    .model(Model::new("gpt-5.6-terra", OpenAI::from_env()?))
    .capability(Jev::new(std::env::var("TYPESAFE_API_KEY")?))
    .build()?;

let session = Engine::new().create(agent);
let turn = session
    .run("Rate this subject line for pushiness: 'Act now before it is too late'")
    .await?;
# Ok(())
# }
```

The agent calls `jev_evaluate`, writing its own questions about whatever it is
looking at, and gets the same calibrated numbers back. It is the identical tool
the hosted [TypeSafe integration](/integrations/typesafe/) gives platform
agents — same name, same schema — so behavior matches whether you embed the
Framework or run on Everruns.

Which one to reach for:

| | you decide | the agent decides |
|---|---|---|
| **who asks** | your code writes the questions | the model writes the questions |
| **use** | `Classifier` | the `Jev` capability |
| **good for** | a policy check, a routing rule, a gate | verification inside a longer task |

## What stays with an agent

A classification owns no session, no history, and no workspace, and runs no
tools.
Reach for an [agent](/framework/agents/) as soon as the work needs any of those.
Typed output guarantees the interface, not the truth: validate thresholds
against your own data and consequences.

## Testing without a credential

`Classifier::simulated` returns a fixed number from an in-process stub. It is a
**test double**, not a local classifier: it runs no inference, reads nothing
from the state you pass it, and is not a way to classify without a provider. It
exists so tests and examples can assert on the code around a classification
without a network call or an API key.

Real work always goes through a classifier service — `TypeSafe` above, or
your own `ClassifierService`.

```rust
use everruns::Classifier;

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let p = Classifier::simulated(0.93)
    .probability("Does this convey urgency?", "Two hours on hold.")
    .await?;
assert!(p > 0.9);
# Ok(())
# }
```

Because the answer is fixed, a simulated classification proves your threshold
logic runs — never that a real classifier would return that number.

The runnable version of this page is
[`direct_classification.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/direct_classification.rs).
It uses the stub by default so it runs with no key; pass `--live` (with
`--features typesafe` and `TYPESAFE_API_KEY` set) to send the same questions to a
real classifier. [`agent_classification.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/agent_classification.rs)
does the same for the agent path.
