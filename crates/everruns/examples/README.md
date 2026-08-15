# `everruns` examples

These examples use the application-facing [`everruns`](../README.md) crate.
Most use `gpt-5.6-terra` and require `OPENAI_API_KEY`;
`capability_configuration`, `canonical_events`, `live_session`, `session_work`,
`workspace_policy`, and `session_history` run entirely offline.
`workspace_heads` also runs offline against a local Git repository.

| Example | What it demonstrates | Run |
|---|---|---|
| [`capability_configuration.rs`](capability_configuration.rs) | One open entrypoint for typed Compaction and ToolSearch, a code-defined Definition, and a dynamic vendor reference | `cargo run -p everruns --example capability_configuration` |
| [`workspace_policy.rs`](workspace_policy.rs) | Safe read/write scopes, default restrictions, and trusted starter files | `cargo run -p everruns --example workspace_policy` |
| [`live_session.rs`](live_session.rs) | Non-blocking send, automatic steering, and optional waiting | `cargo run -p everruns --example live_session` |
| [`hello.rs`](hello.rs) | Minimal agent, typed tool, turn result, and event observation | `cargo run -p everruns --features openai --example hello` |
| [`production_agent.rs`](production_agent.rs) | Tool safety boundary and production-shaped multi-turn use | `cargo run -p everruns --features openai --example production_agent` |
| [`github_monitor.rs`](github_monitor.rs) | Host-owned background work that wakes an agent when it finishes | `cargo run -p everruns --features openai --example github_monitor -- --simulate` |
| [`session_work.rs`](session_work.rs) | Session-owned work, leased delivery, and completion wakes | `cargo run -p everruns --example session_work` |
| [`session_history.rs`](session_history.rs) | Durable local resume and bounded, event-derived history pages | `cargo run -p everruns --features local --example session_history` |
| [`engine_sessions.rs`](engine_sessions.rs) | Concrete Engine ownership, isolated sessions, and engine-scoped resume | `cargo run -p everruns --example engine_sessions` |
| [`workspace_heads.rs`](workspace_heads.rs) | Isolated Git-worktree heads, Environments, and durable session binding | `cargo run -p everruns --features local --example workspace_heads -- /path/to/repo /path/to/state` |
| [`canonical_events.rs`](canonical_events.rs) | Lossless recording and typed rendering of live canonical events | `cargo run -p everruns --example canonical_events` |
| [`subagents.rs`](subagents.rs) | Concurrent child agents managed by an application-owned task registry | `cargo run -p everruns --features openai --example subagents` |
| [`observe_and_cancel.rs`](observe_and_cancel.rs) | Live event streaming and cooperative cancellation | `cargo run -p everruns --features openai --example observe_and_cancel` |
| [`advanced_capability.rs`](advanced_capability.rs) | Curated capability SPI with typed protocol, metadata, progress, and structured errors | `cargo run -p everruns --features openai --example advanced_capability` |
| [`lifecycle_hooks.rs`](lifecycle_hooks.rs) | Awaited agent, turn, tool, and completion handlers | `cargo run -p everruns --features openai --example lifecycle_hooks` |

The GitHub monitor's default `--simulate` mode skips GitHub access but still
uses the configured model. Its explicit live mode requires an authenticated
GitHub CLI:

```text
cargo run -p everruns --features openai --example github_monitor -- --live OWNER/REPO PR_NUMBER
```
