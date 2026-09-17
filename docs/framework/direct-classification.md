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
use everruns::{Classifier, TypeSafeClassifier};

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let judge = Classifier::new(TypeSafeClassifier::from_env()?);
let spam = judge
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

A judgment asks one or more questions about the same state. Each is one of
three shapes:

```rust
use everruns::Classifier;

# async fn run(judge: Classifier) -> Result<(), Box<dyn std::error::Error>> {
let answers = judge
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

## Offline by default

`Classifier::simulated` needs no credentials and no network, so judgments are
testable the same way agents and completions are:

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

## Credentials

`TypeSafeClassifier::from_env()` reads your application's own
`TYPESAFE_API_KEY`, and requires the `jev` feature:

```toml
everruns = { version = "0.22", features = ["jev"] }
```

A deployment running the Everruns platform configures a separate
`UTILITY_TYPESAFE_API_KEY` for its [guardrails](/capabilities/guardrails/) — a
different account from the one an embedding application holds.

## What stays with an agent

A judgment owns no session, no history, and no workspace, and runs no tools.
Reach for an [agent](/framework/agents/) as soon as the work needs any of those.
Typed output guarantees the interface, not the truth: validate thresholds
against your own data and consequences.

Runnable: [`direct_classification.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/direct_classification.rs).
