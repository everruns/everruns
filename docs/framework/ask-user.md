---
title: Answer an agent's questions
description: Implement the AskUser trait so an embedding application can answer an agent's structured questions from its own interface.
sidebar:
  order: 8
---

An agent with the [Ask User](/capabilities/ask-user/) capability can ask the person it is working with a small batch of structured questions and wait for the answer. In a hosted product the browser renders that card. In an embedding application there is no browser, so the application answers — which is what the `AskUser` trait is for.

```rust
use everruns::ask_user::{Answer, AnsweredBy, AskUser, Outcome, Question, Status, async_trait};
use everruns::{Agent, Model};

struct HouseRules;

#[async_trait]
impl AskUser for HouseRules {
    async fn ask(&self, questions: &[Question]) -> Outcome {
        let answers = questions
            .iter()
            .map(|question| Answer {
                id: question.id.clone().unwrap_or_default(),
                selected: question
                    .options
                    .iter()
                    .find(|option| option.is_default)
                    .or_else(|| question.options.first())
                    .map(|option| option.label.clone())
                    .into_iter()
                    .collect(),
                other_text: None,
                secret_ref: None,
            })
            .collect();

        Outcome {
            status: Status::Answered,
            answered_by: AnsweredBy::Unattended,
            answers,
        }
    }
}

let agent = Agent::builder()
    .instructions("Confirm deployment choices before acting.")
    .model(Model::simulated("Done."))
    .ask_user(HouseRules)
    .build()?;
# Ok::<(), everruns::BuildError>(())
```

`AgentBuilder::ask_user` registers the responder and enables the capability in one call. The responder runs **inside** the tool call, so the turn never parks waiting for an external result — the agent asks, your code answers, and the turn continues.

## Route by session

A host that serves several sessions overrides `ask_in` instead of relying on
the default, which ignores the context and calls `ask`. `AskContext` carries
the session id, the `ask_user` tool call id, and the turn id when known.

```rust
use everruns::ask_user::{AskContext, AskUser, DefaultsResponder, Outcome, Question, async_trait};

struct Router;

#[async_trait]
impl AskUser for Router {
    // Only reached when a caller has no context to give.
    async fn ask(&self, questions: &[Question]) -> Outcome {
        DefaultsResponder.ask(questions).await
    }

    async fn ask_in(&self, context: &AskContext, questions: &[Question]) -> Outcome {
        # let _ = (context, questions);
        // Send `questions` to the client that owns `context.session_id()`.
        # unimplemented!()
    }
}
```

## Without a responder

`.capability("ask_user")` on its own uses `DefaultsResponder`: it applies the options the model marked as recommended, falls back to the first option, and reports `AnsweredBy::Unattended`. Headless runs resolve immediately rather than waiting out the timeout for somebody who is not there.

A text or secret question has no unattended value. `DefaultsResponder` declines
a batch containing either kind rather than returning an empty answer.

## Report who answered

`answered_by` is part of the contract, not decoration:

| Value | Meaning |
|---|---|
| `User` | A person actually chose this |
| `Timeout` | The deadline passed and a default was applied |
| `Unattended` | Nobody could be asked; a default was applied |

Report `User` only when a person really answered. An agent that reads a fallback as a considered choice acts with more confidence than the answer earns, and that is the failure this field exists to prevent.

## A worked responder

[`examples/weekend-concierge-host`](https://github.com/everruns/everruns/tree/main/examples/weekend-concierge-host) implements `TerminalResponder` over stdin: numbered options, comma-separated toggles for a multi-select, a free-text path when the question allows one, and terminal echo turned off for a credential.

```
[Energy] How much energy does the group have on Friday?
  *1. Up for anything — Games, noise, moving around.
   2. Low-key — Sitting, talking, snacks.
   3. Something else
>
```

Two details in it are worth copying into any responder:

**An empty answer takes the declared default** rather than returning nothing. A question the model asked and nobody addressed is something it cannot distinguish from a deliberate skip.

**A secret answer carries a reference, never a value.** `Answer` has no `value` field at all, so there is no path from a collected credential into the transcript. The host keeps the value; the agent gets `session:MY_TOKEN` and tools resolve it by name.

**A text answer carries its string in `other_text`.** It has no selected option:

```rust
Answer {
    id,
    selected: Vec::new(),
    other_text: Some("feature/open-question".to_string()),
    secret_ref: None,
}
```

```rust
Answer {
    id,
    selected: Vec::new(),
    other_text: None,
    secret_ref: Some(everruns::ask_user::session_secret_ref("MY_TOKEN")),
}
```

For the smallest possible version, [`crates/everruns/examples/ask_user.rs`](https://github.com/everruns/everruns/tree/main/crates/everruns/examples/ask_user.rs) runs a responder and the unattended path side by side and prints what each decided.

## Questions are not permission

`ask_user` auto-resolves, so it is for decisions and preferences only. A destructive, irreversible, or outward-facing action needs `request_approval`, whose wait does not auto-resolve. See [the boundary](/capabilities/ask-user/#not-a-consent-gate).

## See Also

- [Ask User capability](/capabilities/ask-user/), the contract and its limits
- [Configure and author capabilities](/framework/advanced-capabilities/)
- [Lifecycle hooks](/framework/lifecycle-hooks/), for intercepting tool calls rather than answering them
