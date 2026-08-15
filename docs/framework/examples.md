---
title: Runnable Examples
description: Explore complete Framework programs maintained and compiled with the everruns crate.
---

The [`crates/everruns/examples` catalog](https://github.com/everruns/everruns/tree/main/crates/everruns/examples)
contains the maintained public examples. Each imports the `everruns` facade.

| Example | Demonstrates | Command |
| --- | --- | --- |
| [`capability_configuration.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/capability_configuration.rs) | Typed Compaction and ToolSearch, a code-defined Definition, and a dynamic third-party reference through one entrypoint | `cargo run -p everruns --example capability_configuration` |
| [`workspace_policy.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/workspace_policy.rs) | Safe workspace scopes and trusted starter files, fully offline | `cargo run -p everruns --example workspace_policy` |
| [`live_session.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/live_session.rs) | Non-blocking send, automatic steering, and optional waiting, fully offline | `cargo run -p everruns --example live_session` |
| [`hello.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/hello.rs) | Small live-provider agent | `cargo run -p everruns --features openai --example hello` |
| [`production_agent.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/production_agent.rs) | Tools, files, and production-style setup | `cargo run -p everruns --features openai --example production_agent` |
| [`github_monitor.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/github_monitor.rs) | Typed tools and an offline simulation mode | `cargo run -p everruns --features openai --example github_monitor -- --simulate` |
| [`session_work.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/session_work.rs) | Offline session work, leased delivery, and completion wakes | `cargo run -p everruns --example session_work` |
| [`session_history.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/session_history.rs) | Offline durable resume and bounded history pages | `cargo run -p everruns --features local --example session_history` |
| [`canonical_events.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/canonical_events.rs) | Offline lossless recording and typed rendering of live events | `cargo run -p everruns --example canonical_events` |
| [`subagents.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/subagents.rs) | Public facade composition for delegated work | `cargo run -p everruns --features openai --example subagents` |
| [`observe_and_cancel.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/observe_and_cancel.rs) | Live events and cancellation | `cargo run -p everruns --features openai --example observe_and_cancel` |
| [`advanced_capability.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/advanced_capability.rs) | Code-defined capability through the unified `capability(...)` entrypoint | `cargo run -p everruns --features openai --example advanced_capability` |
| [`lifecycle_hooks.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/lifecycle_hooks.rs) | Awaited agent, turn, tool, and completion handlers | `cargo run -p everruns --features openai --example lifecycle_hooks` |

Live-provider modes use `gpt-5.6-terra` and require `OPENAI_API_KEY`.
`capability_configuration`, `canonical_events`, `live_session`, `session_work`,
`workspace_policy`, and `session_history` are fully offline;
the GitHub monitor also offers a simulated GitHub flow:

```bash
cargo run -p everruns --features openai --example github_monitor -- --simulate
```

For copyable command details and behavior notes, use the
[`examples/README.md`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/README.md)
next to the source. Examples that demonstrate low-level host internals remain
advanced-host examples, not alternative Framework entrypoints.
