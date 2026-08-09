---
title: Lifecycle Hooks
description: Run typed application handlers at agent, turn, tool, and completion boundaries.
---

Lifecycle hooks run trusted application code at defined execution boundaries. Register
them on `Agent::builder()` when work must finish before execution can continue, or when
an application needs a typed failure from a lifecycle action.

```rust
use everruns::prelude::*;

let agent = Agent::builder()
    .instructions("You are concise.")
    .model(Model::simulated("Ready."))
    .on_agent_start(|context| async move {
        println!("starting {}", context.session_id);
    })
    .on_turn_start(|context| async move {
        if context.input.content.is_empty() {
            Err("empty input")
        } else {
            Ok(())
        }
    })
    .on_completion(|context| async move {
        println!("turn stopped with {:?}", context.turn.stop_reason);
    })
    .build()?;
# let _ = agent;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Handlers are async `Fn` closures. An infallible handler returns `()`; a fallible handler
returns `Result<(), E>` where `E` implements `Display`. Wrap synchronous work in an async
block, such as `|context| async move { record(context) }`.

## Hooks or events?

Hooks and [session events](/framework/events-and-cancellation/) serve different jobs.

| | Lifecycle hooks | Session events |
| --- | --- | --- |
| Purpose | Extend execution with application behavior | Observe execution for UI, telemetry, or logs |
| Delivery | Awaited at a lifecycle boundary | Non-blocking stream from `Session::events()` |
| Effect on a run | A pre-effect error may prevent its scoped work | Never changes or delays a run |
| Registration | `AgentBuilder::on_*` | Subscribe on each `Session` |

Do not register a hook merely to mirror the event feed. Use hooks when ordering or failure
semantics matter; use events for observation.

## Lifecycle points

Handlers at one lifecycle point run sequentially in builder registration order.

| Builder method | Runs | Error behavior |
| --- | --- | --- |
| `on_agent_start` | Before the first turn attempted by each session | The first error returns `RunError::Hook`; the next run retries the complete chain |
| `on_turn_start` | Before every turn enters the runtime | The first error returns `RunError::Hook` and prevents that turn |
| `on_tool_start` | Before a model-requested tool call executes | The first error blocks only that call, skips later start handlers for it, and records a `HookFailure` |
| `on_tool_end` | After a tool call reaches a terminal result, including a blocked call | Errors are isolated, recorded, and do not skip later end handlers |
| `on_completion` | After a non-cancelled runtime turn reaches a terminal outcome | Errors are isolated, recorded, and do not skip later completion handlers |

`Session::inspect()` may materialize a runtime but invokes no lifecycle handler. A successful
agent-start chain runs once for that session. If it fails or is cancelled, the next run starts
the complete chain again, so external agent-start effects should be idempotent.

Tool-start runs after any earlier execution gates configured by the host. Tool-end runs for
every call that reaches a terminal result, including a call blocked by an earlier gate; in that
case the Framework tool-start handler might not have run. Independent calls in a parallel tool
batch can run their hook chains concurrently.

Completion receives terminal `Turn` values whether `turn.success` is true or false. A runtime
error that produces no `Turn`, and a cancelled in-flight turn, do not run completion handlers.
Every completion handler receives the same pre-completion `CompletionContext` snapshot.

## Failures and execution effects

Hook contexts are owned, read-only snapshots. A hook cannot rewrite input, tool arguments,
tool results, or the returned turn. Errors affect execution only where work has not happened:

- agent-start and turn-start errors prevent the turn and return `RunError::Hook`;
- tool-start errors prevent that one tool call and appear in `Turn::hook_failures`;
- tool-end and completion errors cannot roll back completed work, so they are isolated in
  `Turn::hook_failures` and the remaining handlers still run.

Each `HookFailure` identifies the lifecycle point and its zero-based handler index. Tool
failures also identify the tool and call. A tool-start error shown to the model is deliberately
generic; the handler's detailed message remains application-facing on `HookFailure`.

With no registered hooks, execution behavior is unchanged and `Turn::hook_failures` is empty.

## Cancellation, concurrency, and panics

A token cancelled before `run_with` skips all handlers. Cancellation during agent-start,
turn-start, or an in-flight tool chain drops the active handler future, skips the remaining
turn work, and does not run completion. Side effects that finished before cancellation are not
rolled back. The synthesized cancelled `Turn` does not report partial failures from the dropped
in-flight hook chain.

Once the runtime commits a turn, completion handlers finish in order even if that run token is
then cancelled. This makes post-turn delivery predictable.

The same `Fn` handlers are shared by every session. Separate sessions, and separate calls in a
parallel tool batch, may invoke a handler concurrently. Protect shared mutable state inside the
closure and do not depend on failure ordering across parallel tool calls.

The Framework adds no hook timeout and does not catch panics. Apply an application timeout
inside a handler when external work must be bounded; let ordinary Rust panic behavior handle
programming defects.

## Sensitive data

Lifecycle handlers are trusted in-process application code. Turn and tool contexts can contain
user input, model-selected arguments, tool results, or backend error text. Do not log or export
whole contexts without applying the same redaction and access controls as the underlying data.

## Runnable example

The focused [`lifecycle_hooks.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/lifecycle_hooks.rs)
example registers all five lifecycle points around a typed tool:

```bash
cargo run -p everruns --features openai --example lifecycle_hooks
```

It uses `gpt-5.6-terra` and requires `OPENAI_API_KEY`.
