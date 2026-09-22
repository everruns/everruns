# Weekend Concierge Host

A standalone application example for the `everruns` Framework API.

This example lives in the root `examples/` folder on purpose: it behaves like a
small external host application, not like an internal crate-local demo.

## What It Shows

- an application-defined function tool with proprietary local data
- **a host answering the agent's questions** — an `AskUser` responder reading a
  real terminal, which is the half of `ask_user` an embedding host owns
- seeded workspace files available inside a Framework session
- a deterministic in-process turn driven by `llmsim`
- how to inspect the resulting context and observe session events

## Credentials

None. The model is `llmsim`, the venue data is a hardcoded list in `src/lib.rs`,
and nothing leaves the process. No provider key, no network.

## Run

The agent asks two questions before it plans anything, and waits for you:

```bash
cargo run --manifest-path examples/weekend-concierge-host/Cargo.toml
```

To run it without a person — CI, a pipeline, a quick check — use the same
answers the offline test uses:

```bash
cargo run --manifest-path examples/weekend-concierge-host/Cargo.toml -- --scripted
```

## Test

```bash
cargo test --manifest-path examples/weekend-concierge-host/Cargo.toml
```

## What Happens

The example hosts a tiny "weekend concierge" app:

- the agent asks the group two questions with `ask_user` — one single-select
  (energy) and one multi-select (must-haves) — before looking anything up
- the host provides a `lookup_neighborhood_spot` tool
- the agent gets seeded `/workspace/welcome-note.md` and `/workspace/weekend-brief.md` files
- the Framework session handles the full `input -> reason -> act` loop

The console output prints:

- the answers the host supplied, read back out of the emitted `tool.completed`
  event, so it prints what actually reached the model rather than what the
  responder believes it said
- the seeded brief file
- the tools visible to the runtime
- the final response
- the message transcript
- the emitted event types, including `tool.completed`

## Contrasting Scenarios

| Answer at the prompt | What reaches the model |
| --- | --- |
| `1` then `1,2` | `Up for anything`, `Snacks`, `Desserts` |
| `2` then `3` | `Low-key`, `Quiet corner` |
| *(empty)* then *(empty)* | `Up for anything`, `Snacks` — the marked default on the first question, and the first option on the second, which marks none |
| `a rooftop` then `3` | `a rooftop (typed)`, `Quiet corner` — free text, because the question allows `Something else` |

Piping works too, which is how the run above stays scriptable:

```bash
printf '2\n1,3\n' | cargo run --manifest-path examples/weekend-concierge-host/Cargo.toml
```

## The Terminal Responder

`src/terminal.rs` implements `everruns::ask_user::AskUser` over stdin: numbered
options, comma-separated toggles for a multi-select, a free-text path when the
question allows one, and **echo off** for a `kind: "secret"` question.

A secret answer carries a `session:{name}` reference and never the value — the
credential stays in the host's own keeping, so it cannot reach the transcript,
the event log, or model context. `a_secret_answer_carries_no_value` pins that.

It lives here rather than in `demo-support` on purpose: that crate promises its
observers never change what an agent does, and a responder decides what the
agent is told. It moves there only if it stops being the thing that answers.

## Limits

- `llmsim` drives the model, so the *plan* is fixed; the offline tests exercise
  tool and responder behavior, not model quality. Your answers change what the
  model is told, not which venue this script ends on.
- Echo off uses `termios` and is a no-op when stdin is not a terminal, which is
  what makes the piped form above work.
- The default engine is in-memory, not durable storage.

The binary stays thin on purpose. The host wiring lives in `src/lib.rs`, so the
same deterministic flow is exercised by the automated test and the runnable
example — `run_weekend_concierge` takes the responder, and only who answers
differs between them.
