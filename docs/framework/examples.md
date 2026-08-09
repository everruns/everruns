---
title: Runnable Examples
description: Explore complete Framework programs maintained and compiled with the everruns crate.
---

The [`crates/everruns/examples` catalog](https://github.com/everruns/everruns/tree/main/crates/everruns/examples)
contains the maintained public examples. Each imports the `everruns` facade.

| Example | Demonstrates | Command |
| --- | --- | --- |
| [`hello.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/hello.rs) | Small live-provider agent | `cargo run -p everruns --features openai --example hello` |
| [`production_agent.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/production_agent.rs) | Tools, files, persistence compatibility, and production-style setup | `cargo run -p everruns --features openai,jsonl --example production_agent` |
| [`github_monitor.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/github_monitor.rs) | Typed tools and an offline simulation mode | `cargo run -p everruns --features openai --example github_monitor -- --simulate` |
| [`subagents.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/subagents.rs) | Public facade composition for delegated work | `cargo run -p everruns --features openai --example subagents` |
| [`observe_and_cancel.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/observe_and_cancel.rs) | Live events and cancellation | `cargo run -p everruns --features openai --example observe_and_cancel` |

Live modes use `gpt-5.6-terra` and require `OPENAI_API_KEY`. The GitHub monitor's
simulation mode is the credential-free entrypoint:

```bash
cargo run -p everruns --features openai --example github_monitor -- --simulate
```

For copyable command details and behavior notes, use the
[`examples/README.md`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/README.md)
next to the source. Examples that demonstrate low-level host internals remain
runtime compatibility examples, not alternative Framework entrypoints.
