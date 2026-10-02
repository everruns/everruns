# everruns-ag-ui

> Rust types for the AG-UI 1.0 agent-to-application protocol.

[![crates.io](https://img.shields.io/crates/v/everruns-ag-ui.svg)](https://crates.io/crates/everruns-ag-ui)
[![docs.rs](https://docs.rs/everruns-ag-ui/badge.svg)](https://docs.rs/everruns-ag-ui)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

Part of the [Everruns](https://everruns.com) ecosystem. Everruns uses it to
expose agents to AG-UI front ends such as CopilotKit and to call AG-UI agents.

[AG-UI](https://docs.ag-ui.com) is the open event protocol between an agent
and the application that renders it: the application posts a `RunAgentInput`,
the agent streams back events (text, tool calls, reasoning, activity,
subagents) and ends the run with an outcome. This crate is a dependency-light
(`serde` only) implementation of the 1.0 wire format.

## Quick example

```rust
use everruns_ag_ui::{Event, Interrupt, RunFinishedEvent, RunFinishedOutcome};

let finished = Event::RunFinished(RunFinishedEvent::new("thread-1", "run-1").with_outcome(
    RunFinishedOutcome::Interrupt {
        interrupts: vec![Interrupt::new("int-1", "tool_approval")],
    },
));
let json = serde_json::to_value(&finished).unwrap();
assert_eq!(json["outcome"]["interrupts"][0]["reason"], "tool_approval");
```

## What It Provides

- Every 1.0 event as one tagged `Event` enum, including run outcomes
  (success, interrupt, cancelled), token usage, reasoning, activity and
  subagent events.
- `RunAgentInput` with frontend tools, context, forwarded props and resume
  entries for interrupts.
- Messages and multimodal content parts (text, image, audio, video, document;
  data, URL or provider file sources).
- The `AgentCapabilities` declaration.
- Serialization that omits absent fields, and tolerant deserialization that
  ignores unknown fields and reads a whole-field `null` as absent.
- The consumer side (`consumer` module): the 1.0 processing model, the
  sequencing rules, `*_CHUNK` expansion and a `RunResult` with messages, tool
  calls, outcome, interrupts and usage; `ResumeBuilder` enforces the resume
  coverage rule.
- An HTTP/SSE client for AG-UI agents behind the `client` feature.

## Features

| Feature | Adds |
|---|---|
| `client` | `client::AgUiClient`: POST a `RunAgentInput`, stream checked events (pulls in `reqwest`) |
| `core` | `projection`: Everruns runtime events as an AG-UI run (Everruns-internal) |

## Design notes

The upstream 1.0 JSON Schema, fixture corpus and client conformance corpus
are vendored in the repository's `spec/` (MIT, from
[ag-ui-protocol/ag-ui](https://github.com/ag-ui-protocol/ag-ui)). The test
suite parses every fixture, validates everything these types serialize
against the schema, and replays every conformance stream through the
consumer.

## Documentation

- [AG-UI 1.0 specification](https://docs.ag-ui.com/spec/1.0)
- [API reference on docs.rs](https://docs.rs/everruns-ag-ui)
- [Everruns docs](https://docs.everruns.com)

## License

[MIT](https://github.com/everruns/everruns/blob/main/LICENSE)
