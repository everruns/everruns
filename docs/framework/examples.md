---
title: Runnable Examples
description: Explore complete Framework programs maintained and compiled with the everruns crate.
---

The [`crates/everruns/examples` catalog](https://github.com/everruns/everruns/tree/main/crates/everruns/examples)
contains the maintained public examples. Each imports the `everruns` facade.

## Complete agents

The root-level [`examples`](https://github.com/everruns/everruns/tree/main/examples)
catalog contains five Framework walkthroughs. Each folder includes the program,
instructions, fixtures where applicable, and recording scripts. Run them from a
repository checkout: their dependencies point to the workspace crates.

`cargo run` uses a real provider and can incur charges. CI tests offline tool
behavior and recording logic; it does not establish the quality of a live model's answer.

| Example | Provider and model | What it does |
| --- | --- | --- |
| [Support Agent](/framework/examples/support-agent/) | OpenAI `gpt-5.6-terra` | Chooses between MFA recovery, lockout, and browser troubleshooting from facts and policy. |
| [Everruns Support Agent](/framework/examples/everruns-support-agent/) | Anthropic `claude-opus-5` | Searches and reads citable official documentation snapshots. |
| [Coding Review Agent](/framework/examples/coding-review-agent/) | Anthropic `claude-sonnet-5` | Reads a refund contract and executes a fixed regression before reporting a defect. |
| [Research Agent](/framework/examples/research-agent/) | OpenRouter `z-ai/glm-5.2` | Searches and fetches primary sources before writing a cited brief. |
| [Incident Commander Agent](/framework/examples/incident-commander-agent/) | Meta Model API `muse-spark-1.3` | Investigates fixture telemetry and persists an evidence-backed incident update. |

Start with Support for typed tools, Research for reusable capabilities, or Code
Review for restricted execution. Each walkthrough shows the agent builder and
session loop, explains expected behavior, and documents what remains a fixture.

These are in-memory sessions. For durability itself, use the session-history and
workspace examples below. Importable hosted Platform definitions live separately
in [`examples/agents`](https://github.com/everruns/everruns/tree/main/examples/agents).

## Execution runtimes

| Example | Provider and model | What it does |
| --- | --- | --- |
| [Bashkit Repo Agent](/framework/examples/bashkit-repo-agent/) | OpenAI `gpt-5.6-terra` | Cuts a release in a real repository with the sandboxed Bashkit shell as its only tool, then verifies the result on disk. |

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
