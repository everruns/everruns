# Weekend Concierge Host

A standalone application example for the `everruns` Framework API.

This example lives in the root `examples/` folder on purpose: it behaves like a
small external host application, not like an internal crate-local demo. It is
also excluded from the repository workspace, so it consumes `everruns` by path
the way a real downstream crate would.

## What It Shows

- an application-defined function tool with proprietary local data
- **a terminal `AskUser` responder**: numbered options for a single-select,
  comma-separated toggles for a multi-select, a free-text "Other…" path, and an
  unechoed prompt for a credential
- the agent asking a two-question set *before* it plans, because a concierge
  that never asks about preferences is a bad concierge
- seeded workspace files available inside a Framework session
- a deterministic in-process turn driven by `llmsim`
- how to inspect the resulting context and observe session events

## Credentials

None. The model is `llmsim`, so the example runs entirely offline and incurs no
provider charges. The only secret involved is the one *you* type if you exercise
the `secret` question path, and it never leaves this process.

## Run

```bash
cargo run --manifest-path examples/weekend-concierge-host/Cargo.toml
```

## Test

```bash
cargo test --manifest-path examples/weekend-concierge-host/Cargo.toml
```

## What Happens

The example hosts a tiny "weekend concierge" app:

- the host provides a `lookup_neighborhood_spot` tool and an `AskUser` responder
- the agent asks two questions — one single-select (`Vibe`) and one
  multi-select (`Musts`) — in a single batch, so the group is interrupted once
- the host answers them, the agent looks up a venue, and then it recommends one
- the agent gets seeded `/workspace/welcome-note.md` and
  `/workspace/weekend-brief.md` files
- the Framework session handles the full `input -> reason -> act` loop

## Contrasting Scenarios

| Run it… | What answers the questions | `answered_by` |
| --- | --- | --- |
| In a terminal | You do, at the prompts | `user` |
| Piped, in CI, or under `cargo test` | The declared defaults | `unattended` |
| With `.capability("ask_user")` and no responder | `DefaultsResponder` | `unattended` |

The third row is what the focused
[`ask_user`](../../crates/everruns/examples/ask_user.rs) example shows in
isolation.

## Expected Outcome

The console output prints the seeded brief, the tools visible to the runtime
(`ask_user` and `lookup_neighborhood_spot`), the final response, the message
transcript, and the emitted event types. The turn takes three iterations and two
tool calls. The transcript shows the `ask_user` result carrying the selections
and `answered_by`, then the venue lookup, then the recommendation.

## Secrets

`kind: "secret"` questions are answered with a **reference, never a value**. The
responder reads the credential with terminal echo disabled, hands it to the
host's own store, and returns `session:NAME`. Nothing in the tool result, the
event log, or the model's context holds the credential itself — an `ask_user`
answer is a tool result, so a value there would be plaintext in both.

## Limits

- Echo is disabled with `stty`, which is POSIX-only. On a terminal where that
  fails, the responder says so rather than silently echoing what you type.
- The host secret store is a plain in-memory map standing in for the encrypted
  session-secret store a hosted deployment has.
- Offline tests exercise tool and responder behavior, not model quality: the
  model is scripted, so the recommendation is fixed.
- The default engine is in-memory, not durable storage.
- The terminal responder is kept in this crate rather than in
  `examples/demo-support`. Depending on an in-workspace helper would undo the
  point of this example being standalone; if a second example needs one, that is
  the moment to lift it.

The binary stays thin on purpose. The host wiring lives in `src/lib.rs` and the
responder in `src/terminal.rs`, so the same deterministic flow is exercised by
the automated test and the runnable example — the test supplies a scripted
responder, `cargo run` supplies the terminal one.
