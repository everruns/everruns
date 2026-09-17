# everruns-integrations-typesafe

> Typed judgments from [TypeSafe](https://typesafe.ai)'s System One model — for
> Rust, and for Everruns agents.

[![Crates.io](https://img.shields.io/crates/v/everruns-integrations-typesafe.svg)](https://crates.io/crates/everruns-integrations-typesafe)
[![Documentation](https://docs.rs/everruns-integrations-typesafe/badge.svg)](https://docs.rs/everruns-integrations-typesafe)
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
# Standalone TypeSafe client — no Everruns dependency.
everruns-integrations-typesafe = { version = "0.1", default-features = false }

# Client plus the Everruns capability and hosted connector.
everruns-integrations-typesafe = "0.1"
```

| Feature | Brings in |
|---|---|
| *(none)* | `TypeSafeClient`, `Evaluation`, `Question`, `Judgment`. `reqwest` + `serde` only. |
| `capability` | The `typesafe` Everruns capability, its `typesafe_evaluate` tool, and `TypeSafe` for `AgentBuilder::capability`. |
| `hosted` *(default)* | `capability`, plus inventory registration and the connector-catalog entry. |
| `integration` | Compiles the real-API smoke tests. |

## Quick start

```rust,no_run
use everruns_integrations_typesafe::{Evaluation, Question, TypeSafeClient};

# async fn run() -> Result<(), everruns_integrations_typesafe::Error> {
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

## As an Everruns capability

With default features the crate contributes the `typesafe` capability, which
gives an agent one tool, `typesafe_evaluate`. The agent hands it content and
its own typed questions, and gets calibrated numbers back instead of forming a
second impression in prose:

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

The key is resolved from the user's TypeSafe connection, falling back to the
`TYPESAFE_API_KEY` session secret. For an embedded agent, hand it in directly
with `TypeSafe::new(key)` — the credential stays inside the client and never
enters capability config, metadata, or `Debug` output.

The same client also backs Everruns' `guardrails` capability when a check sets
`"engine": "judgment"`, which turns one round trip per policy into one round
trip per stage. See
[`knowledge/execution/guardrails.md`](../../knowledge/execution/guardrails.md).

## Examples

```sh
export TYPESAFE_API_KEY=...
cargo run -p everruns-integrations-typesafe --example joke_judge
cargo run -p everruns-integrations-typesafe --example triage
cargo run -p everruns-integrations-typesafe --example screen_message "text to screen"
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

- [API reference (docs.rs)](https://docs.rs/everruns-integrations-typesafe)
- [TypeSafe documentation](https://docs.typesafe.ai)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
