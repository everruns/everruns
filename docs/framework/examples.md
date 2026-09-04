---
title: Runnable Examples
description: Explore complete Framework programs maintained and compiled with the everruns crate.
---

The [`crates/everruns/examples` catalog](https://github.com/everruns/everruns/tree/main/crates/everruns/examples)
contains the maintained public examples. Each imports the `everruns` facade.

## Complete agents

The root-level [`examples/agents`](https://github.com/everruns/everruns/tree/main/examples/agents)
catalog pairs importable Platform definitions with five self-contained Framework
programs. Each `cargo run` uses a real provider and model; CI only constructs
the agents with placeholder credentials, so it never makes billed calls.

| Example | Provider and model | What it does |
| --- | --- | --- |
| [Support Agent](/framework/examples/support-agent/) | OpenAI `gpt-5.6-terra` | Looks up safe customer state and answers support questions. |
| [Everruns Support Agent](/framework/examples/everruns-support-agent/) | Anthropic `claude-opus-5` | Troubleshoots Framework questions with documentation links. |
| [Coding Review Agent](/framework/examples/coding-review-agent/) | Anthropic `claude-sonnet-5` | Reads and reviews a self-contained code change. |
| [Research Agent](/framework/examples/research-agent/) | OpenRouter `z-ai/glm-5.2` | Uses typed Brave web search, cites sources, and reports uncertainty. |
| [Incident Commander Agent](/framework/examples/incident-commander-agent/) | Meta Model API `muse-spark-1.3` | Records an incident update and coordinates safe next actions. |

Each example page includes source, prerequisite, command, and terminal
screencast.

## Core crate catalog

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
| [`engine_sessions.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/engine_sessions.rs) | Concrete Engine ownership, isolated sessions, and engine-scoped resume | `cargo run -p everruns --example engine_sessions` |
| [`workspace_heads.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/workspace_heads.rs) | Isolated Git workspace heads, Environment binding, and durable reopening | `cargo run -p everruns --features local --example workspace_heads -- /path/to/repo /path/to/state` |
| [`canonical_events.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/canonical_events.rs) | Offline bounded recording and typed rendering of live events | `cargo run -p everruns --example canonical_events` |
| [`subagents.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/subagents.rs) | Public facade composition for delegated work | `cargo run -p everruns --features openai --example subagents` |
| [`observe_and_cancel.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/observe_and_cancel.rs) | Live events and cancellation | `cargo run -p everruns --features openai --example observe_and_cancel` |
| [`advanced_capability.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/advanced_capability.rs) | Code-defined capability through the unified `capability(...)` entrypoint | `cargo run -p everruns --features openai --example advanced_capability` |
| [`lifecycle_hooks.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/lifecycle_hooks.rs) | Awaited agent, turn, tool, and completion handlers | `cargo run -p everruns --features openai --example lifecycle_hooks` |

Live-provider modes use `gpt-5.6-terra` and require `OPENAI_API_KEY`.
`capability_configuration`, `canonical_events`, `engine_sessions`,
`live_session`, `session_work`, `workspace_heads`, `workspace_policy`, and
`session_history` are fully offline;
the GitHub monitor also offers a simulated GitHub flow:

```bash
cargo run -p everruns --features openai --example github_monitor -- --simulate
```

For copyable command details and behavior notes, use the
[`examples/README.md`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/README.md)
next to the source. Examples that demonstrate low-level host internals remain
advanced-host examples, not alternative Framework entrypoints.
