---
title: Open Mic Agent
description: Giving an agent the classifier as a capability, and keeping the decision in written thresholds.
---

[Browse the complete example](https://github.com/everruns/everruns/tree/main/examples/open-mic-agent).

Book tonight's open mic at a fictional comedy club. The agent never decides whether a bit is funny by taste: it measures the material with a classifier, then applies the club's numeric house rules to pick a slot — 🎤 main stage, 🌙 late slot, or 📋 the open list.

![Open Mic Agent terminal demo](https://raw.githubusercontent.com/everruns/everruns/main/examples/open-mic-agent/demo/demo.gif)

## What you learn

Giving an agent the `Jev` classifier as a capability, letting it write its own typed questions, and keeping the decision in written thresholds instead of the model's opinion.

## Scenario and expected outcome

Four fictional submissions, one per outcome. The agent reads the bit, asks four questions in a single `jev_evaluate` call — `laugh` and `clean` (noul, a probability), `style` (choice), `polish` (score) — and then reads `src/resources/house-rules.md`.

| Submission | The bit | Measured `laugh` / `clean` | Expected slot |
| --- | --- | --- | --- |
| `mic_pun` | A clean cascade of bakery puns | 0.46 / 0.86 | 🎤 Main stage |
| `mic_late` | Funny, but drinking is the punchline | 0.60 / 0.07 | 🌙 Late slot: `laugh` holds, `clean` does not |
| `mic_flat` | An office in-joke about a printer named Gerald | 0.22 / 0.81 | 📋 Open list: clean, but a room of strangers will not laugh |
| `mic_long` | Nine great minutes about owls | 0.64 / 0.93 | 📋 Open list: five minutes is a hard cap, whatever it measures |

Those numbers are from an actual run against `jev-1.13.0`, and they are why the house rules ask for `laugh` ≥ 0.35 rather than a round-sounding 0.90. A `noul` near 0.5 means yes and no are near-equally likely, not "medium", and a stranger laughing out loud at a written pun is a genuinely uncertain proposition. Thresholds are calibrated against the material they will judge; picking one that sounds strict just sends everybody to the open list. Live answers move a little between runs, so treat the table as calibration data, not as a fixture.

`mic_long` is the interesting one. Its numbers qualify it for the main stage and it still does not get booked, because set length is a fact from the submission, not a judgment for the classifier. Numbers rank material; they do not repeal the rules around it.

## Run it

Install Rust/Cargo, clone the repository, and run from its root. These folders are self-contained **within the workspace**: their Cargo manifests reference the local Framework crates, so copying one folder alone is not sufficient.

```bash
git clone https://github.com/everruns/everruns.git
cd everruns
export ANTHROPIC_API_KEY="your-key"
export TYPESAFE_API_KEY="your-key"
cargo run -p everruns-open-mic-agent
```

Two credentials, two jobs: Anthropic runs the agent, TypeSafe answers its questions. The configured model is `claude-sonnet-5`, and the classifier is `jev-latest`, TypeSafe's alias for the current Jev — pin an exact id such as `jev-1.13.0` once a threshold is calibrated, so a vendor update cannot move it under you. Provider access and funded credits are required; a model identifier alone does not grant access. Keep keys in your environment, not in source control. Missing variables, provider errors, or unsuccessful turns exit nonzero.

Try the contrasting submissions:

```bash
cargo run -p everruns-open-mic-agent -- "Book mic_late into tonight's line-up. Measure the bit before you decide."
cargo run -p everruns-open-mic-agent -- "Book mic_flat into tonight's line-up. Measure the bit before you decide."
cargo run -p everruns-open-mic-agent -- "Book mic_long into tonight's line-up. Measure the bit before you decide."
```

Or enter one interactively:

```bash
cargo run -p everruns-open-mic-agent -- --interactive
```

## Build the agent

The agent definition lives in `src/agent.rs`; `main.rs` only handles input and runs the session. The prompt, the submissions, and the house rules live under `src/resources/`. The two local tools supply evidence; `Jev` supplies judgment as numbers.

```rust
pub fn build(provider: impl Into<Provider>, jev: Jev) -> Result<Agent, BuildError> {
    Agent::builder()
        .name("open-mic-agent")
        .instructions(include_str!("resources/instructions.md"))
        .provider(provider)
        .model(MODEL)
        .max_iterations(12)
        .tool(tools::read_submission())
        .tool(tools::read_house_rules())
        .capability(jev)
        .build()
}
```

`Jev::from_env()` reads `TYPESAFE_API_KEY` and contributes one tool, `jev_evaluate`. It is the same tool the hosted [TypeSafe integration](/integrations/typesafe/) gives platform agents, so behavior matches whether you embed the Framework or run on Everruns.

## What the agent measures

`jev_evaluate` takes the content to judge plus a list of questions, and answers them all in one request. The agent writes the questions itself, from the ids and level descriptions the house rules spell out:

```json
{
  "state": "I got fired from the bakery for stealing dough...",
  "questions": [
    {"id": "laugh", "type": "noul", "instructions": "Would a general club audience laugh out loud at this bit?"},
    {"id": "clean", "type": "noul", "instructions": "Is this bit fine for an all-ages 7pm room: no profanity, sex, or drinking as the punchline?"},
    {"id": "style", "type": "choice", "instructions": "What kind of comedy is this?",
     "options": {"pun": null, "observational": null, "storytelling": null, "absurdist": null}},
    {"id": "polish", "type": "score", "instructions": "How ready is this bit for a paying room?",
     "levels": ["A rough first draft that still needs writing", "Works, but runs long between laughs", "Stage-ready, tight from the first line"]}
  ]
}
```

Back comes a probability for each `noul`, a selected option with its distribution for the `choice`, and a position along the levels for the `score` — with the per-level probabilities, which is how the main-stage rule can ask for a *tail* ("works" plus "stage-ready" ≥ 0.70) instead of an average. Something probably tight but possibly a first draft must not average into fine.

Ids are never shown to the model, so each question has to carry its full meaning in its instructions — `"id": "clean"` asks nothing on its own.

Questions in one call cannot see each other's answers, which is why the prompt insists on a single call: four independent judgments about the same bit, one round trip.

## Choosing the classifier path

This example is the agent-decides half of [Direct Classification](/framework/direct-classification/). When your own code knows which questions matter — a routing rule, a policy gate — build a `Classifier` and ask them yourself. Here the agent is holding the material, so it writes the questions and the house rules hold the thresholds.

## Send, observe, and wait

The Framework interaction stays readable in `main.rs`. The shared demo helper subscribes before sending, filters events to this turn, shows bounded tool previews, waits for completion, and rejects unsuccessful turns. It changes presentation only; use `session.send_and_wait(question).await?` when you do not need the live tool timeline.

```rust
let agent = agent::build(
    everruns_anthropic::from_env("anthropic")?,
    everruns::Jev::from_env()?,
)?;
let engine = Engine::new();
let session = engine.create(agent);

println!("MODEL: {}", agent::MODEL);
demo::run(&session, &question).await?;
```

This engine is in-memory: tonight's line-up is not persisted anywhere.

## How the tools work

`read_submission` returns one of four fictional submissions from `src/resources/submissions.json`, including the bit's full text and the set length. `read_house_rules` returns `src/resources/house-rules.md`, which names the four questions to ask and the threshold each slot needs. Neither tool judges anything, and no tool books a real slot.

## Validate the behavior

```bash
cargo test -p everruns-open-mic-agent
bash examples/open-mic-agent/demo/record.sh --check
```

Tests cover the fixtures, rejection of unknown ids, and the house rules staying in sync with the prompt — every question the prompt tells the agent to ask is defined with a threshold. They do not grade the classifier: whether `mic_pun` still measures above 0.35 is a question about the model, answered by running it.

CI runs these offline checks without provider credentials. A calibrated number is a measurement, not a fact about your audience; thresholds like 0.70 belong to this fictional club and should be calibrated against your own material and consequences.

## Demo and recording

The screencast runs the same `cargo run -q -p everruns-open-mic-agent` command shown above. VHS hides most provider wait time but does not replace the model, the classifier, or the tools with scripted output. Read the [captured transcript](https://github.com/everruns/everruns/blob/main/examples/open-mic-agent/demo/transcript.txt) at your own pace.

With credentials exported and VHS, ffmpeg, and a VHS-compatible browser installed:

```bash
bash examples/open-mic-agent/demo/record.sh
```

The recording script uses exported `ANTHROPIC_API_KEY` and `TYPESAFE_API_KEY` when both are present, otherwise Doppler project `everruns-dev`, config `dev`. It runs the real command inside VHS and updates `demo/demo.gif` plus `demo/transcript.txt` only after a successful turn.

## Adapt it

Swap the fixtures for your own queue — submissions, applications, user reports, draft copy — and keep the shape: facts from a tool, judgment from the classifier, thresholds in a rules file a human can edit without touching Rust. Write the questions as the thing you would ask a person, pin an exact model id once a threshold is calibrated, and keep hard constraints like the five-minute cap out of the classifier, where they cannot drift.

## Boundaries

The Rubber Chicken, its house rules, the comics, and their bits are fictional and written for this example. This is a content-triage exercise, not a booking system, and a classifier's opinion of a joke is not an audience's.

## Source map

`src/main.rs`: input and session execution; `src/agent.rs`: agent definition, tools, and the `Jev` capability; `src/tools.rs`: submission and house-rule lookups; `src/resources/`: prompt, submissions, and house rules; `demo/`: live VHS recording, transcript, and recording script. `examples/demo-support` handles shared terminal presentation.
