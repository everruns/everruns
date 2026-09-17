# typesafe-systemone

> Typed judgments from [TypeSafe](https://typesafe.ai)'s System One model — for
> Rust, and for Everruns agents.

[![Crates.io](https://img.shields.io/crates/v/typesafe-systemone.svg)](https://crates.io/crates/typesafe-systemone)
[![Documentation](https://docs.rs/typesafe-systemone/badge.svg)](https://docs.rs/typesafe-systemone)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

A System One model answers *typed questions* about state and returns numbers:
a probability, a selected option, a graded level. It does not write prose, and
it does not explain itself. That makes it a programming primitive — your code
keeps the workflow and asks the model only for the semantic judgment it cannot
compute.

Use it where you would otherwise prompt a chat model and parse JSON out of its
answer: verification, rating, routing, extraction, moderation, reranking.

## Install

```toml
typesafe-systemone = "0.1"
```

No Everruns dependency: this is the TypeSafe client on its own. It is
maintained alongside [Everruns](https://everruns.com), the durable agentic
harness engine, which wraps it as an agent capability in
[`everruns-integrations-typesafe`](https://crates.io/crates/everruns-integrations-typesafe)
— but nothing here requires it.

The `integration` feature compiles the real-API smoke tests; it is off by
default and needs `TYPESAFE_API_KEY`.

## Quick start

```rust,no_run
use typesafe_systemone::{Evaluation, Question, TypeSafeClient};

# async fn run() -> Result<(), typesafe_systemone::Error> {
let client = TypeSafeClient::from_env()?; // TYPESAFE_API_KEY

let judgment = client
    .evaluate(
        Evaluation::new("I told my wife she was drawing her eyebrows too high. She looked surprised.")
            .ask("is_joke", Question::noul("Is this told as a joke?"))
            .ask(
                "humor",
                Question::score(
                    "How funny would a general adult audience find this?",
                    ["Not funny at all", "Mildly amusing", "Genuinely funny", "Hilarious"],
                ),
            ),
    )
    .await?;

let humor = judgment.score("humor")?;
if judgment.noul("is_joke")? > 0.5 && humor.probability_at_or_above(2) > 0.5 {
    println!("{} — ship it", humor.nearest_label().unwrap_or_default());
}
# Ok(())
# }
```

Questions asked together run in parallel inside one request and cannot see each
other's answers. Batching is the cheap path: ask everything the code might
need, including questions only one branch will read.

## What It Provides

- `TypeSafeClient` over the System One endpoint, with a request builder that
  validates before it spends a round trip
- The three question types — `noul`, `choice`, `score` — and their typed answers,
  with the full probability distribution the model returned
- Accessors that turn a distribution into a decision: `probability_at_or_above`,
  `normalized`, `nearest_label`, `confidence`
- Bounded retries for transport failures, `429`, and `5xx`, honoring `Retry-After`
- Errors that never echo your API key, even when the upstream body does

## The three primitives

| You need | Ask | You get back |
|---|---|---|
| Whether a condition holds | `Question::noul` | Probability of yes, 0..1. No separate confidence — the probability *is* the answer. |
| Exactly one option from a set | `Question::choice` | The selected option, the probability of every option, and a confidence derived from that distribution. |
| A degree along a described dimension | `Question::score` | A probability-weighted position across your ordered levels, the per-level distribution, and confidence. |

Two habits that matter more than they look:

- **A noul near 0.5 means yes and no are near-equally likely** — not "medium
  intensity", and not low confidence in a middling verdict.
- **Read the tail, not the mean, for "any serious violation" rules.**
  `ScoreAnswer::probability_at_or_above(level)` exists for exactly that: a
  bimodal answer that is *probably fine, possibly awful* must not average into
  fine. `normalized()` maps the score onto `0.0..=1.0` so a threshold survives
  a change in the number of levels.

Confidence is a second axis: the answer tells you *what*, confidence tells you
whether to act without a person. Typed output guarantees the interface, not the
truth — validate thresholds against your own data and consequences.

## Using it with Everruns

[`everruns-integrations-typesafe`](../../../integrations/typesafe/) wraps this
client as the `typesafe` capability: one tool, `typesafe_evaluate`, that lets an
agent ask its own typed questions about content and get calibrated numbers back
instead of forming a second impression in prose.

```jsonc
{
  "state": "Why did the chicken cross the road? To get to the other side.",
  "questions": [
    { "id": "is_funny", "type": "noul", "instructions": "Would a general audience laugh?" },
    { "id": "humor", "type": "score", "instructions": "How funny is it?",
      "levels": ["Not funny at all", "Mildly amusing", "Genuinely funny", "Hilarious"] }
  ]
}
```

The same client also backs Everruns'
[guardrails](https://docs.everruns.com/capabilities/guardrails/) when a check
sets `"engine": "judgment"`, which turns one round trip per policy into one
round trip per stage.

## Examples

```sh
export TYPESAFE_API_KEY=...
cargo run -p typesafe-systemone --example joke_judge
cargo run -p typesafe-systemone --example triage
cargo run -p typesafe-systemone --example screen_message "text to screen"
```

- [`joke_judge`](examples/joke_judge.rs) — verify and rate content; policy stays in code.
- [`triage`](examples/triage.rs) — fan-out over structured state, with confidence-gated routing.
- [`screen_message`](examples/screen_message.rs) — one request, many hazards, a threshold you own.

## Errors and retries

`evaluate` retries transport failures, `429`, and `5xx` (including `529
Overloaded`) with exponential backoff, honoring `Retry-After` up to a cap.
`401` and `422` are returned immediately — a different key or a different
request is needed, and retrying only spends time. Error messages carry the
status and the upstream `message` field when there is one; raw bodies are never
surfaced, because an upstream error can echo the request headers that carry
your key.

## Documentation

- [API reference (docs.rs)](https://docs.rs/typesafe-systemone)
- [TypeSafe documentation](https://docs.typesafe.ai)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
